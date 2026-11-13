use crate::config::Config;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Output;
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct RawTunnel {
    id: String,
    name: String,
    #[serde(default)]
    connections: Vec<RawConn>,
}

#[derive(Debug, Deserialize)]
struct RawConn {
    #[serde(default)]
    colo_name: String,
}

#[derive(Debug, Clone)]
pub struct Tunnel {
    pub id: String,
    pub name: String,
    pub connections: Vec<Conn>,
}

#[derive(Debug, Clone)]
pub struct Conn {
    pub colo: String,
}

fn is_exec(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Find cloudflared from an explicit path, $CLOUDFLARED, $PATH, or ~/.local/bin.
pub fn find_binary(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && is_exec(p)
    {
        return Some(p.to_path_buf());
    }
    if let Some(p) = env::var_os("CLOUDFLARED") {
        let p = PathBuf::from(p);
        if is_exec(&p) {
            return Some(p);
        }
    }
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            let cand = dir.join("cloudflared");
            if is_exec(&cand) {
                return Some(cand);
            }
        }
    }
    let local = crate::config::home().join(".local/bin/cloudflared");
    if is_exec(&local) {
        return Some(local);
    }
    None
}

pub async fn run(bin: &Path, args: &[&str]) -> Result<Output> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to run {}", bin.display()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "{} {} failed: {}",
            bin.display(),
            args.join(" "),
            stderr.trim()
        );
    }
    Ok(out)
}

pub async fn version(bin: &Path) -> Result<String> {
    let out = run(bin, &["--version"]).await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn list(bin: &Path) -> Result<Vec<Tunnel>> {
    let out = run(bin, &["tunnel", "list", "--output", "json"]).await?;
    let raw: Vec<RawTunnel> =
        serde_json::from_slice(&out.stdout).context("parsing `cloudflared tunnel list` JSON")?;
    Ok(raw
        .into_iter()
        .map(|t| Tunnel {
            id: t.id,
            name: t.name,
            connections: t
                .connections
                .into_iter()
                .map(|c| Conn { colo: c.colo_name })
                .collect(),
        })
        .collect())
}

pub async fn find_tunnel(bin: &Path, name: &str) -> Result<Option<Tunnel>> {
    Ok(list(bin).await?.into_iter().find(|t| t.name == name))
}

pub async fn create(bin: &Path, name: &str) -> Result<Tunnel> {
    if let Some(t) = find_tunnel(bin, name).await? {
        return Ok(t);
    }
    run(bin, &["tunnel", "create", name]).await?;
    for _ in 0..4 {
        if let Some(t) = find_tunnel(bin, name).await? {
            return Ok(t);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    bail!("tunnel `{name}` was created but did not appear in `tunnel list`")
}

pub async fn route_dns(bin: &Path, name: &str, hostname: &str, overwrite: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["tunnel", "route", "dns"];
    if overwrite {
        args.push("--overwrite-dns");
    }
    args.push(name);
    args.push(hostname);
    run(bin, &args).await?;
    Ok(())
}

pub async fn login(bin: &Path) -> Result<()> {
    let status = Command::new(bin)
        .args(["tunnel", "login"])
        .status()
        .await
        .context("running cloudflared tunnel login")?;
    if !status.success() {
        bail!("cloudflared tunnel login failed");
    }
    Ok(())
}

pub fn write_cf_config(cfg: &Config, path: &Path) -> Result<()> {
    let body = format!(
        "tunnel: {}\ncredentials-file: {}\n\ningress:\n  - hostname: {}\n    service: ssh://{}\n  - service: http_status:404\n",
        cfg.tunnel_id,
        cfg.creds().display(),
        cfg.hostname,
        cfg.local_target,
    );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        if existing != body {
            let backup = path.with_extension("yml.bak");
            std::fs::copy(path, backup)?;
        }
    }
    std::fs::write(path, body)?;
    Ok(())
}
