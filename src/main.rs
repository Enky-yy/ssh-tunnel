mod cf;
mod client;
mod config;
mod doctor;
mod net;
mod service;
mod status;
mod termux;
mod ui;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use config::Config;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "ssh-tunnel",
    version,
    about = "Turnkey remote SSH access over Cloudflare Tunnel"
)]
struct Cli {
    /// Machine-readable JSON output where supported
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write the config file (auto-detects an existing setup)
    Init(InitArgs),
    /// Create the tunnel, route DNS, install the service, and verify
    Setup(SetupArgs),
    /// Start (and enable) the tunnel service
    Up,
    /// Stop the tunnel service
    Down,
    /// Restart the tunnel service
    Restart,
    /// Show tunnel, DNS, and service state
    Status,
    /// Check every layer from sshd to the Cloudflare edge
    Doctor(DoctorArgs),
    /// Tail the tunnel service logs
    Logs(LogsArgs),
    /// Print the SSH config for a device, or list registered keys
    Client(ClientArgs),
    /// Print a Termux setup script for Android
    Termux,
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    tunnel: Option<String>,
    #[arg(long)]
    hostname: Option<String>,
    #[arg(long)]
    user: Option<String>,
    #[arg(long, default_value = "127.0.0.1:22")]
    target: String,
    #[arg(long)]
    identity: Option<PathBuf>,
    #[arg(long)]
    cloudflared: Option<PathBuf>,
    /// Accept detected defaults without prompting
    #[arg(short, long)]
    yes: bool,
}

#[derive(Args)]
struct SetupArgs {
    /// Overwrite an existing DNS record for the hostname
    #[arg(long)]
    force_dns: bool,
}

#[derive(Args)]
struct DoctorArgs {
    /// Also attempt a real SSH login through the tunnel
    #[arg(long)]
    deep: bool,
}

#[derive(Args)]
struct LogsArgs {
    #[arg(short, long)]
    follow: bool,
}

#[derive(Args)]
struct ClientArgs {
    #[command(subcommand)]
    cmd: ClientCmd,
}

#[derive(Subcommand)]
enum ClientCmd {
    /// Print a device's SSH config, optionally register its public key
    Add {
        name: String,
        /// Public key file, or `-` for stdin
        #[arg(long)]
        key_file: Option<String>,
        /// Print a QR code for the connection command
        #[arg(long)]
        qr: bool,
        /// IdentityFile to print for the device (default: ~/.ssh/id_ed25519)
        #[arg(long)]
        identity: Option<String>,
    },
    /// List authorized public keys
    List,
}

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ssh_tunnel=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();

    let cli = Cli::parse();
    if let Err(e) = dispatch(cli).await {
        eprintln!("{} {:#}", ui::red("error:"), e);
        std::process::exit(1);
    }
}

async fn dispatch(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.cmd {
        Cmd::Init(a) => cmd_init(a).await,
        Cmd::Setup(a) => cmd_setup(a).await,
        Cmd::Up => {
            let cfg = Config::load_required()?;
            service::enable_now(&cfg).await?;
            println!("{} tunnel up", ui::ok());
            Ok(())
        }
        Cmd::Down => {
            let cfg = Config::load_required()?;
            service::stop(&cfg).await?;
            println!("{} tunnel down", ui::ok());
            Ok(())
        }
        Cmd::Restart => {
            let cfg = Config::load_required()?;
            service::restart(&cfg).await?;
            println!("{} tunnel restarted", ui::ok());
            Ok(())
        }
        Cmd::Status => {
            let cfg = Config::load_required()?;
            status::run(&cfg, json).await
        }
        Cmd::Doctor(a) => {
            let cfg = Config::load_required()?;
            let checks = doctor::run(&cfg, a.deep).await?;
            doctor::print(&checks, json)?;
            if doctor::failed(&checks) {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Logs(a) => {
            let cfg = Config::load_required()?;
            service::logs(&cfg, a.follow).await
        }
        Cmd::Client(a) => match a.cmd {
            ClientCmd::Add {
                name,
                key_file,
                qr,
                identity,
            } => {
                let cfg = Config::load_required()?;
                client::add(&cfg, &name, key_file.as_deref(), qr, identity.as_deref())
            }
            ClientCmd::List => {
                let cfg = Config::load_required()?;
                client::list(&cfg)
            }
        },
        Cmd::Termux => {
            let cfg = Config::load_required()?;
            println!("{}", termux::script(&cfg));
            Ok(())
        }
    }
}

async fn cmd_init(args: InitArgs) -> Result<()> {
    let binary = cf::find_binary(args.cloudflared.as_deref())
        .or_else(|| cf::find_binary(None))
        .unwrap_or_else(|| config::home().join(".local/bin/cloudflared"));

    let existing = Config::load().ok();
    let tunnels = if binary.exists() {
        cf::list(&binary).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let default_tunnel = existing
        .as_ref()
        .map(|c| c.tunnel.clone())
        .or_else(|| {
            tunnels
                .iter()
                .find(|t| !t.connections.is_empty())
                .map(|t| t.name.clone())
        })
        .or_else(|| tunnels.first().map(|t| t.name.clone()))
        .unwrap_or_else(|| "g15".to_string());

    let default_hostname = existing
        .as_ref()
        .map(|c| c.hostname.clone())
        .or_else(detect_hostname)
        .unwrap_or_else(|| "ssh.example.com".to_string());

    let default_user = existing
        .as_ref()
        .map(|c| c.user.clone())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "user".to_string());

    let default_target = existing
        .as_ref()
        .map(|c| c.local_target.clone())
        .unwrap_or_else(|| args.target.clone());

    let default_identity = existing
        .as_ref()
        .and_then(|c| c.identity_file.clone())
        .or_else(detect_identity);

    let tunnel = pick(
        args.tunnel.clone(),
        args.yes,
        "Tunnel name",
        &default_tunnel,
    );
    let hostname = pick(
        args.hostname.clone(),
        args.yes,
        "Hostname",
        &default_hostname,
    );
    let user = pick(args.user.clone(), args.yes, "SSH user", &default_user);
    let target = if args.yes {
        args.target.clone()
    } else {
        ui::prompt("Local SSH target", &default_target)
    };

    let identity = match args.identity.clone() {
        Some(p) => Some(p),
        None if args.yes => default_identity.clone(),
        None => {
            let shown = default_identity
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let value = ui::prompt("Identity file (blank for client default)", &shown);
            if value.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            }
        }
    };

    let tunnel_id = tunnels
        .iter()
        .find(|t| t.name == tunnel)
        .map(|t| t.id.clone())
        .or_else(|| {
            existing
                .as_ref()
                .filter(|c| c.tunnel == tunnel)
                .map(|c| c.tunnel_id.clone())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default();

    let cfg = Config {
        tunnel,
        tunnel_id,
        hostname,
        user,
        local_target: target,
        cloudflared: binary,
        identity_file: identity,
        ssh_alias: None,
    };
    cfg.save()?;

    println!("{} wrote {}", ui::ok(), Config::path().display());
    println!("  tunnel     {}", cfg.tunnel);
    println!("  hostname   {}", cfg.hostname);
    println!("  user       {}", cfg.user);
    println!("  target     {}", cfg.local_target);
    println!("  cloudflared {}", cfg.cloudflared.display());
    if let Some(id) = &cfg.identity_file {
        println!("  identity   {}", id.display());
    }
    println!("\nNext: {}", ui::bold("ssh-tunnel setup"));
    Ok(())
}

async fn cmd_setup(args: SetupArgs) -> Result<()> {
    let mut cfg = Config::load_required()?;
    let bin = cf::find_binary(Some(&cfg.cloudflared))
        .or_else(|| cf::find_binary(None))
        .context(
            "cloudflared not found; install it or run `ssh-tunnel init --cloudflared <path>`",
        )?;
    if cfg.cloudflared != bin {
        cfg.cloudflared = bin.clone();
        cfg.save()?;
    }

    println!("{}", ui::bold("1/5 cloudflare login"));
    if cfg.cert().exists() {
        println!("  {} certificate present", ui::ok());
    } else {
        println!("  a browser will open; pick the zone for {}", cfg.hostname);
        cf::login(&bin).await?;
    }

    println!("{}", ui::bold("2/5 tunnel"));
    let tunnel = cf::create(&bin, &cfg.tunnel).await?;
    println!("  {} {} ({})", ui::ok(), tunnel.name, tunnel.id);
    if cfg.tunnel_id != tunnel.id {
        cfg.tunnel_id = tunnel.id;
        cfg.save()?;
    }

    println!("{}", ui::bold("3/5 dns"));
    if !net::resolve(&cfg.hostname, 443).await.is_empty() && !args.force_dns {
        println!("  {} {} already resolves", ui::ok(), cfg.hostname);
    } else {
        cf::route_dns(&bin, &cfg.tunnel, &cfg.hostname, args.force_dns).await?;
        println!("  {} {} -> {}", ui::ok(), cfg.hostname, cfg.tunnel);
    }

    println!("{}", ui::bold("4/5 config + service"));
    cf::write_cf_config(&cfg, &cfg.cf_config())?;
    service::write_unit(&cfg)?;
    service::daemon_reload().await?;
    service::enable_now(&cfg).await?;
    println!("  {} {}", ui::ok(), cfg.unit_path().display());

    println!("{}", ui::bold("5/5 doctor"));
    let checks = doctor::run(&cfg, false).await?;
    doctor::print(&checks, false)?;
    if doctor::failed(&checks) {
        bail!("doctor found failures; fix them and run `ssh-tunnel doctor`");
    }

    println!(
        "\n{} {} {}",
        ui::ok(),
        ui::bold("ready:"),
        ui::bold(&format!("ssh {}", cfg.alias()))
    );
    Ok(())
}

fn pick(explicit: Option<String>, yes: bool, label: &str, default: &str) -> String {
    match explicit {
        Some(value) => value,
        None if yes => default.to_string(),
        None => ui::prompt(label, default),
    }
}

fn detect_hostname() -> Option<String> {
    let path = config::home().join(".cloudflared").join("config.yml");
    let text = std::fs::read_to_string(path).ok()?;
    let re = regex::Regex::new(r"(?m)^\s*-\s*hostname:\s*(\S+)").ok()?;
    re.captures(&text).map(|c| c[1].to_string())
}

fn detect_identity() -> Option<PathBuf> {
    for name in ["id_ed25519_g15", "id_ed25519", "id_rsa"] {
        let path = config::home().join(".ssh").join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}
