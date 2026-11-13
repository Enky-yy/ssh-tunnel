use crate::config::{self, Config};
use crate::ui;
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Read;

pub fn add(
    cfg: &Config,
    name: &str,
    key_file: Option<&str>,
    qr: bool,
    identity: Option<&str>,
) -> Result<()> {
    if let Some(raw) = read_key(key_file)? {
        if add_key(cfg, &raw)? {
            println!("{} added public key for {}", ui::ok(), name);
        } else {
            println!("{} key already present", ui::warn());
        }
    }

    let block = ssh_block(cfg, identity);
    println!();
    println!("{}", ui::bold(&format!("Client: {name}")));
    println!("Add this to ~/.ssh/config on the device:");
    println!();
    for line in block.lines() {
        println!("  {line}");
    }
    println!();
    println!(
        "Then connect with: {}",
        ui::bold(&format!("ssh {}", cfg.alias()))
    );
    println!();
    println!(
        "{}",
        ui::dim("The device also needs the `cloudflared` binary in PATH.")
    );

    if key_file.is_none() {
        println!();
        println!("No key registered yet. On the device, run:");
        println!("  cat ~/.ssh/id_ed25519.pub");
        println!("Then here:");
        println!("  ssh-tunnel client add {name} --key-file -");
    }

    if qr {
        println!();
        println!("{}", ui::dim("Scan for the connection command:"));
        let cmd = format!(
            "ssh -o ProxyCommand=\"cloudflared access ssh --hostname %h\" {}@{}",
            cfg.user, cfg.hostname
        );
        print_qr(&cmd)?;
    }
    Ok(())
}

pub fn list(cfg: &Config) -> Result<()> {
    let path = cfg.authorized_keys();
    if !path.exists() {
        println!("no authorized_keys at {}", path.display());
        return Ok(());
    }
    let path_str = path.to_str().context("authorized_keys path is not UTF-8")?;
    let out = std::process::Command::new("ssh-keygen")
        .args(["-lf", path_str])
        .output()
        .context("running ssh-keygen")?;
    if !out.status.success() {
        bail!(
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}

fn read_key(key_file: Option<&str>) -> Result<Option<String>> {
    match key_file {
        None => Ok(None),
        Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok(Some(s))
        }
        Some(path) => Ok(Some(
            fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
        )),
    }
}

fn add_key(cfg: &Config, raw: &str) -> Result<bool> {
    let key = raw
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("ssh-") || l.starts_with("ecdsa-") || l.starts_with("sk-"))
        .context("no SSH public key found in input")?;

    let dir = config::home().join(".ssh");
    fs::create_dir_all(&dir)?;
    config::set_mode(&dir, 0o700)?;

    let path = cfg.authorized_keys();
    let mut text = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if text.lines().any(|l| l.trim() == key) {
        return Ok(false);
    }
    text.push_str(key);
    text.push('\n');
    fs::write(&path, text)?;
    config::set_mode(&path, 0o600)?;
    Ok(true)
}

fn ssh_block(cfg: &Config, identity: Option<&str>) -> String {
    let identity = identity.unwrap_or("~/.ssh/id_ed25519");
    format!(
        "Host {alias}\n    HostName {hostname}\n    User {user}\n    IdentityFile {identity}\n    ProxyCommand cloudflared access ssh --hostname %h\n    ServerAliveInterval 30\n    ServerAliveCountMax 3",
        alias = cfg.alias(),
        hostname = cfg.hostname,
        user = cfg.user,
        identity = identity,
    )
}

fn print_qr(text: &str) -> Result<()> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| anyhow::anyhow!("QR encode failed: {e:?}"))?;
    let rendered = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    println!("{rendered}");
    Ok(())
}
