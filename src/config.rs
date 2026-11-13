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
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
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
        fs::create_dir_all(Self::dir())?;
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
