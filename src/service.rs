use crate::config::Config;
use anyhow::{Context, Result, bail};
use tokio::process::Command;

fn uid() -> String {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "1000".to_string())
}

/// `systemctl --user` needs a session bus; make it work from a bare SSH shell too.
fn apply_user_env(cmd: &mut Command) {
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        let uid = uid();
        cmd.env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"));
        cmd.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path=/run/user/{uid}/bus"),
        );
    } else if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
        cmd.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={dir}/bus"));
    }
}

pub fn unit_contents(cfg: &Config) -> String {
    format!(
        "[Unit]\n\
         Description=Cloudflare Tunnel for {tunnel} ({hostname})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart=\"{cf}\" --no-autoupdate --config \"{config}\" tunnel run\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         TimeoutStopSec=10s\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        tunnel = cfg.tunnel,
        hostname = cfg.hostname,
        cf = cfg.cloudflared.display(),
        config = cfg.cf_config().display(),
    )
}

pub fn write_unit(cfg: &Config) -> Result<()> {
    let path = cfg.unit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, unit_contents(cfg))?;
    crate::config::set_mode(&path, 0o644)?;
    Ok(())
}

async fn systemctl(args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user").args(args);
    apply_user_env(&mut cmd);
    let status = cmd.status().await.context("running systemctl --user")?;
    if !status.success() {
        bail!("systemctl --user {} failed", args.join(" "));
    }
    Ok(())
}

async fn systemctl_bool(args: &[&str]) -> bool {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user").args(args);
    apply_user_env(&mut cmd);
    cmd.output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn daemon_reload() -> Result<()> {
    systemctl(&["daemon-reload"]).await
}

pub async fn enable_now(cfg: &Config) -> Result<()> {
    let unit = cfg.unit_name();
    systemctl(&["enable", "--now", unit.as_str()]).await
}

pub async fn restart(cfg: &Config) -> Result<()> {
    let unit = cfg.unit_name();
    systemctl(&["restart", unit.as_str()]).await
}

pub async fn stop(cfg: &Config) -> Result<()> {
    let unit = cfg.unit_name();
    systemctl(&["stop", unit.as_str()]).await
}

pub async fn is_active(cfg: &Config) -> bool {
    let unit = cfg.unit_name();
    systemctl_bool(&["is-active", "--quiet", unit.as_str()]).await
}

pub async fn is_enabled(cfg: &Config) -> bool {
    let unit = cfg.unit_name();
    systemctl_bool(&["is-enabled", "--quiet", unit.as_str()]).await
}

pub async fn logs(cfg: &Config, follow: bool) -> Result<()> {
    let unit = cfg.unit_name();
    let mut cmd = Command::new("journalctl");
    cmd.args(["--user", "-u", unit.as_str()]);
    if follow {
        cmd.arg("-f");
    }
    apply_user_env(&mut cmd);
    let status = cmd.status().await.context("running journalctl")?;
    if !status.success() {
        bail!("journalctl failed");
    }
    Ok(())
}
