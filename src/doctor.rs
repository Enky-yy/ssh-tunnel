use crate::config::Config;
use crate::{cf, net, service, ui};
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
}

impl Check {
    fn ok(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: Status::Ok,
            detail: detail.into(),
        }
    }

    fn warn(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: Status::Warn,
            detail: detail.into(),
        }
    }

    fn fail(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: Status::Fail,
            detail: detail.into(),
        }
    }
}

pub fn failed(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.status == Status::Fail)
}

pub fn print(checks: &[Check], as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(checks)?);
        return Ok(());
    }
    for c in checks {
        let symbol = match c.status {
            Status::Ok => ui::ok(),
            Status::Warn => ui::warn(),
            Status::Fail => ui::fail(),
        };
        println!("{symbol} {:<20} {}", c.name, c.detail);
    }
    Ok(())
}

pub async fn run(cfg: &Config, deep: bool) -> Result<Vec<Check>> {
    let mut checks = Vec::new();

    checks.push(Check::ok("config", Config::path().display().to_string()));

    let bin = cf::find_binary(Some(&cfg.cloudflared));
    match &bin {
        Some(b) => match cf::version(b).await {
            Ok(v) => checks.push(Check::ok("cloudflared", v)),
            Err(e) => checks.push(Check::fail("cloudflared", e.to_string())),
        },
        None => checks.push(Check::fail("cloudflared", "not found")),
    }

    if cfg.cert().exists() {
        checks.push(Check::ok("certificate", "present"));
    } else {
        checks.push(Check::fail(
            "certificate",
            format!("missing {}", cfg.cert().display()),
        ));
    }

    if cfg.creds().exists() {
        checks.push(Check::ok("credentials", cfg.creds().display().to_string()));
    } else {
        checks.push(Check::fail(
            "credentials",
            format!("missing {}", cfg.creds().display()),
        ));
    }

    match fs::read_to_string(cfg.cf_config()) {
        Ok(text) => {
            let has_host = text.contains(&format!("hostname: {}", cfg.hostname));
            let has_service = text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("service: ssh://"))
                .any(|t| normalize_target(t.trim()) == normalize_target(&cfg.local_target));
            if has_host && has_service {
                checks.push(Check::ok("ingress", cfg.cf_config().display().to_string()));
            } else {
                checks.push(Check::fail(
                    "ingress",
                    "config.yml does not match the hostname/target",
                ));
            }
        }
        Err(_) => checks.push(Check::fail(
            "ingress",
            format!("missing {}", cfg.cf_config().display()),
        )),
    }

    let addrs = net::resolve(&cfg.hostname, 443).await;
    if addrs.is_empty() {
        checks.push(Check::fail(
            "dns",
            format!("{} does not resolve", cfg.hostname),
        ));
    } else {
        let ips: Vec<String> = addrs.iter().map(|a| a.ip().to_string()).collect();
        checks.push(Check::ok("dns", ips.join(", ")));
    }

    let active = service::is_active(cfg).await;
    let enabled = service::is_enabled(cfg).await;
    let unit = cfg.unit_name();
    match (active, enabled) {
        (true, true) => checks.push(Check::ok("service", format!("{unit}: active, enabled"))),
        (true, false) => checks.push(Check::warn(
            "service",
            format!("{unit}: active but not enabled"),
        )),
        (false, _) => checks.push(Check::fail("service", format!("{unit}: inactive"))),
    }

    if net::tcp_ok(&cfg.hostname, 443, 8).await {
        checks.push(Check::ok("edge", format!("{}:443 reachable", cfg.hostname)));
    } else {
        checks.push(Check::fail(
            "edge",
            format!("{}:443 unreachable", cfg.hostname),
        ));
    }

    let (local_host, local_port) = net::split_target(&cfg.local_target);
    if net::tcp_ok(&local_host, local_port, 3).await {
        checks.push(Check::ok(
            "sshd",
            format!("listening on {}", cfg.local_target),
        ));
    } else {
        checks.push(Check::fail(
            "sshd",
            format!("nothing listening on {}", cfg.local_target),
        ));
    }

    match fs::read_to_string(cfg.authorized_keys()) {
        Ok(text) => {
            let count = text.lines().filter(|l| !l.trim().is_empty()).count();
            if count == 0 {
                checks.push(Check::fail("authorized_keys", "no keys"));
            } else {
                checks.push(Check::ok("authorized_keys", format!("{count} key(s)")));
            }
        }
        Err(_) => checks.push(Check::fail(
            "authorized_keys",
            format!("missing {}", cfg.authorized_keys().display()),
        )),
    }

    match password_auth(cfg).await {
        Some(true) => checks.push(Check::warn(
            "password auth",
            "still enabled; set PasswordAuthentication no",
        )),
        Some(false) => checks.push(Check::ok("password auth", "disabled (key-only)")),
        None => checks.push(Check::warn("password auth", "could not determine")),
    }

    checks.push(ssh_dir_perms());
    checks.push(authorized_keys_perms(cfg));
    checks.push(secret_perms(cfg));

    if deep {
        match &bin {
            Some(b) => match e2e(cfg, b).await {
                Ok(()) => checks.push(Check::ok("end-to-end", "ssh login OK")),
                Err(e) => checks.push(Check::warn("end-to-end", e.to_string())),
            },
            None => checks.push(Check::warn("end-to-end", "skipped (no cloudflared)")),
        }
    }

    Ok(checks)
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}

fn ssh_dir_perms() -> Check {
    let dir = crate::config::home().join(".ssh");
    match mode_of(&dir) {
        None => Check::warn("ssh dir", format!("missing {}", dir.display())),
        Some(mode) if mode & 0o077 == 0 => {
            Check::ok("ssh dir", format!("{} ({mode:03o})", dir.display()))
        }
        Some(mode) => Check::warn(
            "ssh dir",
            format!("{} is {mode:03o}; run `chmod 700 ~/.ssh`", dir.display()),
        ),
    }
}

fn authorized_keys_perms(cfg: &Config) -> Check {
    let path = cfg.authorized_keys();
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Check::warn("key file perms", format!("{} is a symlink", path.display()))
        }
        Ok(_) => match mode_of(&path) {
            Some(mode) if mode & 0o022 != 0 => Check::fail(
                "key file perms",
                format!("{} is {mode:03o}; run `chmod 600`", path.display()),
            ),
            Some(mode) => Check::ok("key file perms", format!("{mode:03o}")),
            None => Check::warn("key file perms", "could not stat"),
        },
        Err(_) => Check::warn("key file perms", "authorized_keys missing"),
    }
}

fn secret_perms(cfg: &Config) -> Check {
    let mut parts = Vec::new();
    let mut worst = 0;
    for (label, path, secret) in [
        ("credentials", cfg.creds(), true),
        ("cert", cfg.cert(), true),
        ("ingress", cfg.cf_config(), false),
    ] {
        match mode_of(&path) {
            None => {
                parts.push(format!("{label} missing"));
                worst = worst.max(1);
            }
            Some(mode) => {
                let loose = if secret { mode & 0o077 } else { mode & 0o022 };
                if loose != 0 {
                    parts.push(format!("{label} {mode:03o} (too open)"));
                    worst = worst.max(2);
                } else {
                    parts.push(format!("{label} {mode:03o}"));
                }
            }
        }
    }
    let detail = parts.join(", ");
    match worst {
        2 => Check::warn("secret perms", detail),
        1 => Check::warn("secret perms", detail),
        _ => Check::ok("secret perms", detail),
    }
}

fn normalize_target(target: &str) -> (String, u16) {
    let (host, port) = net::split_target(target);
    let host = match host.as_str() {
        "localhost" | "::1" => "127.0.0.1".to_string(),
        other => other.to_string(),
    };
    (host, port)
}

/// Ask sshd which methods it offers without authenticating. "password" in the
/// rejection message means password auth is still enabled.
async fn password_auth(cfg: &Config) -> Option<bool> {
    let target = format!("{}@127.0.0.1", cfg.user);
    let out = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "PreferredAuthentications=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "LogLevel=ERROR",
        ])
        .arg(target)
        .arg("true")
        .output()
        .await
        .ok()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("password") {
        Some(true)
    } else if stderr.contains("publickey") {
        Some(false)
    } else {
        None
    }
}

async fn e2e(cfg: &Config, bin: &Path) -> Result<()> {
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=20",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "LogLevel=ERROR",
    ]);
    cmd.arg("-o");
    cmd.arg(format!(
        "ProxyCommand={} access ssh --hostname {}",
        bin.display(),
        cfg.hostname
    ));
    if let Some(id) = &cfg.identity_file {
        cmd.arg("-o");
        cmd.arg(format!("IdentityFile={}", id.display()));
        cmd.arg("-o").arg("IdentitiesOnly=yes");
    }
    cmd.arg(format!("{}@{}", cfg.user, cfg.hostname));
    cmd.arg("true");

    let out = cmd.output().await?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("Permission denied") {
        anyhow::bail!("auth failed (passphrase-protected key needs ssh-agent)");
    }
    anyhow::bail!(
        "{}",
        stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ssh failed")
    );
}
