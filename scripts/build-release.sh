#!/usr/bin/env bash
# Build the release binary without leaking the builder's home directory and
# username into panic messages. `trim-paths` is still nightly-only, so remap
# the prefix with RUSTFLAGS instead.
set -euo pipefail

cd "$(dirname "$0")/.."

remap_to="${REMAP_TO:-/build}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${HOME}=${remap_to}"

if [ -n "${CARGO_HOME:-}" ] && [ "${CARGO_HOME}" != "${HOME}/.cargo" ]; then
  export RUSTFLAGS="${RUSTFLAGS} --remap-path-prefix=${CARGO_HOME}=${remap_to}/cargo"
fi

exec cargo build --release "$@"
