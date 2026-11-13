#!/usr/bin/env bash
# Host-side hardening for ssh-tunnel. Run as root:
#
#   sudo ./scripts/harden-host.sh
#
# Two changes:
#   1. sshd listens on loopback only (the tunnel forwards to 127.0.0.1:22) and
#      per-source penalties are off, because every tunnel client is 127.0.0.1
#      and an attacker's failed attempts would otherwise lock the owner out.
#   2. ufw actually denies inbound traffic. On Arch/CachyOS `ufw enable` leaves
#      DEFAULT_INPUT_POLICY=ACCEPT, so the rule list alone filters nothing.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "run this with sudo" >&2
  exit 1
fi

dropin=/etc/ssh/sshd_config.d/20-ssh-tunnel.conf
backup="${dropin}.bak"
if [ -f "$dropin" ]; then
  cp -a "$dropin" "$backup"
fi

cat > "$dropin" <<'EOF'
# Managed by ssh-tunnel: only the local tunnel needs this sshd.
ListenAddress 127.0.0.1
# All tunnel clients arrive from 127.0.0.1; per-source penalties would let one
# attacker lock out the owner. Rate limit at the Cloudflare edge instead.
PerSourcePenalties no
EOF

echo "== validating sshd config =="
sshd -t
systemctl reload sshd
echo "sshd reloaded"

if command -v ufw >/dev/null 2>&1; then
  echo "== firewall =="
  ufw default deny incoming
  ufw default allow outgoing
  ufw reload
  ufw status verbose
else
  echo "ufw not installed; skipping firewall changes" >&2
fi

echo
echo "== listeners =="
ss -tlnp | grep -E ':22\b' || true
echo
echo "Verify: ssh-tunnel doctor && ssh <alias>"
