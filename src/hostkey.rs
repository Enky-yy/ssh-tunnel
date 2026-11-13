use crate::config::Config;
use crate::ui;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct HostKey {
    pub kind: String,
    pub fingerprint: String,
    pub blob: String,
}

/// Read the host keys exactly as running sshd presents them. `ssh-keyscan`
/// against loopback needs no root and reflects the live configuration.
fn local_keys() -> Result<Vec<HostKey>> {
    let out = Command::new("ssh-keyscan")
        .args(["-T", "5", "-t", "ed25519,ecdsa,rsa", "127.0.0.1"])
        .output()
        .context("running ssh-keyscan")?;
    if !out.status.success() {
        bail!(
            "ssh-keyscan failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut keys = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let _host = parts.next();
        let kind = parts.next().unwrap_or_default().to_string();
        let blob = parts.next().unwrap_or_default().to_string();
        if kind.is_empty() || blob.is_empty() {
            continue;
        }
        let fingerprint = fingerprint_of(&format!("{kind} {blob}"))?;
        keys.push(HostKey {
            kind,
            fingerprint,
            blob,
        });
    }
    if keys.is_empty() {
        bail!("sshd did not return a host key on 127.0.0.1:22");
    }
    Ok(keys)
}

fn fingerprint_of(key_line: &str) -> Result<String> {
    let mut child = Command::new("ssh-keygen")
        .args(["-l", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running ssh-keygen")?;
    child
        .stdin
        .as_mut()
        .context("ssh-keygen stdin")?
        .write_all(format!("{key_line}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "ssh-keygen rejected the host key: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string())
}

/// Is this host key already pinned in the caller's known_hosts?
/// `None` means this key type has no entry at all, so nothing is wrong.
fn pinned(cfg: &Config, kind: &str, blob: &str) -> Option<bool> {
    let known = crate::config::home().join(".ssh").join("known_hosts");
    if !known.exists() {
        return None;
    }
    let out = Command::new("ssh-keygen")
        .args(["-F", &cfg.hostname, "-f"])
        .arg(&known)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut seen_kind = false;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let _host = parts.next();
        let entry_kind = parts.next().unwrap_or_default();
        let entry_blob = parts.next().unwrap_or_default();
        if entry_kind != kind {
            continue;
        }
        seen_kind = true;
        if entry_blob == blob {
            return Some(true);
        }
    }
    if seen_kind { Some(false) } else { None }
}

pub fn run(cfg: &Config, as_json: bool) -> Result<()> {
    let keys = local_keys()?;

    if as_json {
        let entries: Vec<_> = keys
            .iter()
            .map(|k| {
                serde_json::json!({
                    "type": k.kind,
                    "fingerprint": k.fingerprint,
                    "pinned": pinned(cfg, &k.kind, &k.blob),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "hostname": cfg.hostname,
                "host_keys": entries,
            }))?
        );
        return Ok(());
    }

    println!("{}", ui::bold(&format!("Host keys for {}", cfg.hostname)));
    println!();
    for key in &keys {
        println!("  {:<20} {}", key.kind, ui::bold(&key.fingerprint));
        match pinned(cfg, &key.kind, &key.blob) {
            Some(true) => println!("  {:<20} {}", "", ui::green("pinned in known_hosts")),
            Some(false) => println!(
                "  {:<20} {}",
                "",
                ui::red("MISMATCH: known_hosts has a different key for this host")
            ),
            None => println!("  {:<20} {}", "", ui::yellow("not in known_hosts yet")),
        }
    }
    println!();
    println!(
        "{}",
        ui::dim("On a new device, compare this before accepting the key:")
    );
    println!(
        "  {}",
        ui::dim(&format!(
            "ssh-keyscan -t ed25519 {} | ssh-keygen -lf -",
            cfg.hostname
        ))
    );
    println!();
    println!(
        "{}",
        ui::dim("First connect pins the key (StrictHostKeyChecking accept-new).")
    );
    Ok(())
}
