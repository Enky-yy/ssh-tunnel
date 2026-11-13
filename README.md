# ssh-tunnel

[![CI](https://github.com/Enky-yy/ssh-tunnel/actions/workflows/ci.yml/badge.svg)](https://github.com/Enky-yy/ssh-tunnel/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

**Remote SSH into a machine behind NAT — no VPS, no port forwarding, no inbound ports.**

`ssh-tunnel` is a small CLI that wires up [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/)
for SSH. Your machine dials out to Cloudflare; any device with `ssh` and
`cloudflared` can then reach it from anywhere on port 443.

```
client  ──TLS 443──►  Cloudflare edge  ◄──outbound──  cloudflared  ──►  sshd :22
```

Works on locked-down networks (campus, hotel, CGNAT) because the server side
only makes outbound connections.

## Why

If your machine sits behind a NAT you don't control, you cannot forward port 22.
Cloudflare Tunnel solves the networking problem. This tool solves the twelve
manual steps around it: account login, tunnel creation, DNS, ingress config,
systemd unit, verification, and per-device onboarding — plus a `doctor`
command that tells you *which* layer is broken.

## Features

- **One-time setup** — `ssh-tunnel setup` does login, tunnel, DNS, service, and verification.
- **Layered diagnostics** — `doctor` checks 11 independent layers, not just "is it up".
- **No inbound ports** — only outbound 443/QUIC from the host.
- **Key-only by design** — SSH encryption and host-key verification stay end to end; Cloudflare only relays ciphertext.
- **Per-device onboarding** — prints the exact SSH config, registers a public key from stdin, optional QR.
- **Android ready** — `ssh-tunnel termux` emits a Termux bootstrap script.
- **Systemd user service** — starts on login, restarts on failure.
- **JSON output** — `--json` for `status` and `doctor`, easy to script.
- **Idempotent** — safe to re-run `setup`.

## Requirements

| Requirement | Notes |
|---|---|
| Linux with `systemd --user` | Service management is systemd-specific |
| [`cloudflared`](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) | Any recent version |
| OpenSSH server | `sshd` listening on `127.0.0.1:22` (default) |
| A domain managed by Cloudflare | Free plan is fine |
| SSH keypair | Password auth should be disabled |

## Install

From source:

```bash
git clone https://github.com/Enky-yy/ssh-tunnel.git
cd ssh-tunnel
cargo build --release
install -Dm755 target/release/ssh-tunnel ~/.local/bin/ssh-tunnel
```

Or with Cargo:

```bash
cargo install --path .
```

Make sure `~/.local/bin` (or `~/.cargo/bin`) is in your `PATH`.

## Quick start

```bash
ssh-tunnel init          # detect domain, tunnel, user, key
ssh-tunnel setup         # login, create tunnel, route DNS, install service
ssh-tunnel doctor        # verify every layer
ssh-tunnel status
```

`init` is interactive. Use `ssh-tunnel init --yes` to accept detected defaults
(useful when re-running against an existing setup).

When `setup` finishes, it prints:

```
✓ ready: ssh <alias>
```

## Commands

| Command | Description |
|---|---|
| `init [--yes]` | Write `~/.config/ssh-tunnel/config.toml` |
| `setup [--force-dns]` | Create tunnel, route DNS, write ingress + systemd unit, start, verify |
| `up` / `down` / `restart` | Control `cloudflared-<tunnel>.service` |
| `status [--json]` | Tunnel ID, edge connections, DNS, service state |
| `doctor [--deep] [--json]` | Check config → cert → ingress → DNS → service → edge → sshd → auth |
| `logs [-f]` | Tail the user service journal |
| `client add <name> [--key-file -] [--qr] [--identity]` | Print a device's SSH config; register its public key |
| `client list` | Fingerprints from `authorized_keys` |
| `fingerprint [--json]` | Show host key fingerprints and whether they match `known_hosts` |
| `termux` | Print an Android/Termux setup script |

## Onboarding a device

On the new device:

```bash
ssh-keygen -t ed25519 -C "pixel"
cat ~/.ssh/id_ed25519.pub
```

On the host:

```bash
ssh-tunnel client add pixel --key-file -   # paste the public key
ssh-tunnel client add pixel --qr           # print the SSH block + a QR code
```

Add the printed `Host` block to the device's `~/.ssh/config`, then:

```bash
ssh <alias>
```

### Android (Termux)

Install Termux from F-Droid (not the Play Store), then:

```bash
ssh-tunnel termux | bash
```

The script installs `openssh` and `cloudflared`, creates a device key, appends a
marked `Host` block to `~/.ssh/config` (backing up the old file), and prints the
public key to register.

## How it works

1. `cloudflared` on the host holds a persistent outbound connection to Cloudflare.
2. A DNS record (`ssh.example.com`) points at the Cloudflare edge.
3. A client runs `cloudflared access ssh --hostname ssh.example.com` as its SSH `ProxyCommand`.
4. The edge forwards the stream down the tunnel to the host's `cloudflared`.
5. `cloudflared` connects to `127.0.0.1:22`, where `sshd` authenticates the key.

SSH runs end to end; Cloudflare cannot read the session or impersonate the host
without breaking host-key verification.

## Configuration

`~/.config/ssh-tunnel/config.toml`:

```toml
tunnel = "tunnel_name"
tunnel_id = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
hostname = "ssh.example.com"
user = "alice"
local_target = "127.0.0.1:22"
cloudflared = "/home/alice/.local/bin/cloudflared"
identity_file = "/home/alice/.ssh/id_ed25519"
```

| Field | Meaning |
|---|---|
| `tunnel` | Cloudflare tunnel name |
| `tunnel_id` | UUID, filled in by `setup` |
| `hostname` | Public SSH hostname |
| `user` | Unix user that `sshd` authenticates |
| `local_target` | Where `cloudflared` forwards (`ssh://<target>`) |
| `cloudflared` | Path to the binary |
| `identity_file` | Host's key, used only by `doctor --deep` |
| `ssh_alias` | Optional alias override (defaults to `tunnel`) |

Files the tool manages:

| Path | Purpose |
|---|---|
| `~/.cloudflared/config.yml` | Ingress: hostname → `ssh://127.0.0.1:22` |
| `~/.cloudflared/<uuid>.json` | Tunnel credentials (`chmod 400`) |
| `~/.cloudflared/cert.pem` | Account certificate (`chmod 600`) |
| `~/.config/systemd/user/cloudflared-<tunnel>.service` | Service unit |
| `~/.ssh/authorized_keys` | Authorized public keys |

## What `doctor` checks

| Check | Failure means |
|---|---|
| `config` | `init` was never run |
| `cloudflared` | Binary missing or broken |
| `certificate` | `cloudflared tunnel login` not done |
| `credentials` | Tunnel credentials missing |
| `ingress` | `config.yml` hostname/target mismatch |
| `dns` | Hostname does not resolve |
| `service` | Unit inactive or not enabled |
| `edge` | Hostname:443 unreachable |
| `sshd` | Nothing listening on `local_target` |
| `authorized_keys` | No keys registered |
| `password auth` | Password login still enabled |
| `ssh dir` | `~/.ssh` is group/world accessible (sshd will refuse it) |
| `key file perms` | `authorized_keys` is group/world writable or a symlink |
| `secret perms` | Tunnel credentials readable by other users |
| `end-to-end` (`--deep`) | Real SSH login failed |

## Security model

- **Key-only SSH.** The tool warns if `PasswordAuthentication` is still enabled.
- **No credential storage.** Keys are never embedded or copied; the CLI only prints paths and appends public keys.
- **Least exposure.** Only `localhost:22` is exposed through the tunnel.
- **End-to-end SSH.** Cloudflare relays ciphertext; host keys still verify the server.
- **Local secrets.** `cert.pem` (600), tunnel credentials (400), and config (600).
- **Client IPs are hidden.** `sshd` sees every tunnel connection as `127.0.0.1`, so `fail2ban` cannot distinguish clients. Use key-only auth and `MaxAuthTries`.
- Add [Cloudflare Access](https://developers.cloudflare.com/cloudflare-one/policies/access/)
  in front of the hostname if you want an identity gate.

### Input handling

Config values are written into a systemd unit, a YAML file, an `ssh_config`
block, and a shell script, so each field is checked against a strict allowlist
before use (`tunnel`, `tunnel_id`, `hostname`, `user`, `local_target`, `ssh_alias`).
Newlines, quotes, shell metacharacters, and path separators are rejected, which
rules out unit/YAML/shell injection from a tampered `config.toml`.

`client add` treats a pasted key as untrusted:

- exactly one line starting with `ssh-`, `ecdsa-`, or `sk-`
- no control characters and no `command=`/`from=`/quote prefixes
- validated by `ssh-keygen -l -f`, and the fingerprint is printed so you can compare it on the device
- refuses to write if `~/.ssh/authorized_keys` is a symlink or not a regular file
- input is capped at 64 KiB

`cloudflared` discovery only trusts `$CLOUDFLARED` and `$PATH` for non-root
callers, so an elevated shell cannot be tricked into running a different binary.
The Termux script appends a marked `Host` block (with a backup) instead of
overwriting an existing `~/.ssh/config`.

## Hardening checklist

The tunnel removes the need for inbound ports, but the host still needs its own
baseline. These are the things this tool cannot do for you:

**Firewall.** On Arch/CachyOS, `ufw enable` does not change the default policy:

```bash
grep DEFAULT_INPUT_POLICY /etc/default/ufw   # ACCEPT means nothing is filtered
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw reload
```

Nothing needs to be allowed for the tunnel: `cloudflared` dials out, and
loopback traffic bypasses the firewall. Check what else is listening with
`ss -tlnp`. `scripts/harden-host.sh` applies this along with the sshd changes
below (run it with `sudo`).

**Bind sshd to loopback.** The tunnel forwards to `127.0.0.1:22`, so sshd does
not need to be on the LAN. In `/etc/ssh/sshd_config.d/10-hardening.conf`:

```
ListenAddress 127.0.0.1
```

**Verify the host key.** `ssh.harsh-shah.me` is public, and clients trust the
key on first use. Before connecting from a new device:

```bash
ssh-tunnel fingerprint          # on the host
ssh-keyscan -t ed25519 ssh.harsh-shah.me | ssh-keygen -lf -   # on the device
```

Compare the SHA256 values. `fingerprint` also warns when `known_hosts` holds a
different key for the hostname.

**Do not enable `PerSourcePenalties`.** Through the tunnel every client appears
as `127.0.0.1`, so a single attacker's failed attempts would penalise the
loopback address and lock you out. The same reason makes `fail2ban` ineffective
here; rate limit at the Cloudflare edge instead.

**Optional: Cloudflare Access.** Adding an Access application (with a service
token for `cloudflared access ssh`) puts an identity gate and rate limiting in
front of sshd. It costs every client a token, so it is opt-in.

**Credentials.** `~/.cloudflared/cert.pem` is zone-level: whoever holds it can
create tunnels and rewrite DNS for the domain. It is needed for
`tunnel list/create/route`, not for running the tunnel, so re-run
`cloudflared tunnel login` only when managing, and rotate it if it leaks. Use a
passphrase on the host key (`ssh-keygen -p -f ~/.ssh/id_ed25519`), and keep
`~/.cloudflared` at `700` with credentials at `400`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `lookup … no such host` right after `setup` | Stale resolver cache: `resolvectl flush-caches` |
| `Permission denied (publickey)` | Key not in `authorized_keys`, or wrong `IdentityFile` |
| `cloudflared: command not found` on client | Install `cloudflared` (Termux: `pkg install cloudflared`) |
| Service dies when you log out over SSH | `sudo loginctl enable-linger $USER` |
| QUIC blocked on a network | Add `--protocol http2` to the unit's `ExecStart` |
| `doctor --deep` warns about auth | Passphrase key needs `ssh-add` (BatchMode cannot prompt) |
| Tunnel works, DNS does not | Wait a minute, then `ssh-tunnel doctor` |

## Limitations

- **Linux + systemd only.** No launchd/Windows service support.
- **SSH only.** This is not a VPN; only `local_target` is exposed.
- **Quick tunnels are unsupported.** `trycloudflare.com` quick tunnels reject SSH (`websocket: bad handshake`), so named tunnels are required.
- **Single ingress rule.** `setup` rewrites `~/.cloudflared/config.yml` (backing up the old file) and does not merge multiple rules.
- **Free Tunnel has no SLA.** Expect occasional reconnects.
- **Host sleep breaks sessions.** No wake-on-LAN behind NAT.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
./scripts/build-release.sh
```

`scripts/build-release.sh` remaps `$HOME` out of the binary's panic paths, so
the release does not carry the builder's username. CI runs the same checks plus
`cargo audit` on every push and pull request.

## License

[MIT](LICENSE) © 2026 Harsh Shah
