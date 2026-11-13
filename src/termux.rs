use crate::config::Config;
use std::fmt::Write;

pub fn script(cfg: &Config) -> String {
    let alias = cfg.alias();
    let mut s = String::new();

    writeln!(s, "#!/data/data/com.termux/files/usr/bin/bash").unwrap();
    writeln!(s, "set -euo pipefail").unwrap();
    writeln!(s).unwrap();
    writeln!(s, "pkg update -y").unwrap();
    writeln!(s, "pkg install -y openssh cloudflared").unwrap();
    writeln!(s).unwrap();
    writeln!(
        s,
        "# One key per device; never copy the laptop's private key"
    )
    .unwrap();
    writeln!(
        s,
        "test -f ~/.ssh/id_ed25519 || ssh-keygen -t ed25519 -C \"{alias}\""
    )
    .unwrap();
    writeln!(s).unwrap();
    writeln!(s, "mkdir -p ~/.ssh && chmod 700 ~/.ssh").unwrap();
    writeln!(s, "cat > ~/.ssh/config <<'EOF'").unwrap();
    writeln!(s, "Host {alias}").unwrap();
    writeln!(s, "    HostName {}", cfg.hostname).unwrap();
    writeln!(s, "    User {}", cfg.user).unwrap();
    writeln!(s, "    IdentityFile ~/.ssh/id_ed25519").unwrap();
    writeln!(s, "    ProxyCommand cloudflared access ssh --hostname %h").unwrap();
    writeln!(s, "    ServerAliveInterval 30").unwrap();
    writeln!(s, "    ServerAliveCountMax 3").unwrap();
    writeln!(s, "EOF").unwrap();
    writeln!(s, "chmod 600 ~/.ssh/config").unwrap();
    writeln!(s).unwrap();
    writeln!(s, "echo").unwrap();
    writeln!(
        s,
        "echo \"Public key (add it with: ssh-tunnel client add <name> --key-file -):\""
    )
    .unwrap();
    writeln!(s, "cat ~/.ssh/id_ed25519.pub").unwrap();
    writeln!(s).unwrap();
    writeln!(s, "echo").unwrap();
    writeln!(s, "echo \"Then run: ssh {alias}\"").unwrap();
    writeln!(
        s,
        "echo \"Tip: termux-wake-lock keeps Termux alive in the background\""
    )
    .unwrap();

    s
}
