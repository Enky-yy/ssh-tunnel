# ssh-tunnel

Turnkey remote SSH access to a machine behind NAT, using Cloudflare Tunnel.
No VPS, no port forwarding, no inbound ports.

The laptop runs `cloudflared` as a user service and dials out to Cloudflare.
Clients connect to `ssh.<your-domain>` on 443 with a stock `ssh` client plus
`cloudflared` as a `ProxyCommand`.

```
client  ──TLS 443──►  Cloudflare edge  ◄──outbound──  cloudflared  ──►  sshd :22
```

## Install

```bash
cargo build --release
install -Dm755 target/release/ssh-tunnel ~/.local/bin/ssh-tunnel
```

`cloudflared` must be installed and logged in (`ssh-tunnel setup` will prompt).

## Quick start

```bash
ssh-tunnel init        # detect domain/hostname/tunnel, write config
ssh-tunnel setup       # login, create tunnel, route DNS, install service
ssh-tunnel doctor      # verify every layer
ssh-tunnel status
```

## Commands

| Command | What it does |
|---|---|
| `init [--yes]` | Write `~/.config/ssh-tunnel/config.toml` |
| `setup [--force-dns]` | Create tunnel, route DNS, write config + systemd unit, start |
| `up` / `down` / `restart` | Control `cloudflared-<tunnel>.service` |
| `status [--json]` | Tunnel, connections, DNS, service state |
| `doctor [--deep] [--json]` | Check config → DNS → service → edge → sshd → auth |
| `logs [-f]` | `journalctl --user -u cloudflared-<tunnel>` |
| `client add <name> [--key-file -] [--qr] [--identity]` | Print SSH config, register a public key |
| `client list` | Fingerprints from `authorized_keys` |
| `termux` | Print an Android/Termux setup script |

## Typical onboarding

On the new device:

```bash
ssh-keygen -t ed25519 -C "pixel"
cat ~/.ssh/id_ed25519.pub
```

On the laptop:

```bash
ssh-tunnel client add pixel --key-file -   # paste the key
ssh-tunnel client add pixel --qr           # optional QR
```

On the device, add the printed `Host` block to `~/.ssh/config`, then `ssh <alias>`.

## Security

- SSH stays key-only; the tunnel does not terminate SSH.
- Cloudflare only sees SSH ciphertext; host keys still verify end-to-end.
- `sshd` sees every tunnel connection as `127.0.0.1`, so `fail2ban` cannot
  distinguish clients. Use key-only auth and `MaxAuthTries`.
- Add Cloudflare Access later if you want an identity gate in front of SSH.
- Quick tunnels (`trycloudflare.com`) do **not** support SSH, so this tool
  only uses named tunnels on your own zone.

## Files

| Path | Purpose |
|---|---|
| `~/.config/ssh-tunnel/config.toml` | Tunnel name, hostname, user, paths |
| `~/.cloudflared/config.yml` | Cloudflare ingress (`ssh://127.0.0.1:22`) |
| `~/.cloudflared/<uuid>.json` | Tunnel credentials |
| `~/.config/systemd/user/cloudflared-<tunnel>.service` | Tunnel service |
