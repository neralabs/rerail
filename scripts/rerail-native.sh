#!/usr/bin/env bash
#
# `just rerail-native` — build Rerail's native rules engine for this machine.
#
# The engine is a Rust cdylib loaded through napi; without native/*.node every
# Rerail path that embeds or parses fails with "missing native addon". Published
# packages carry prebuilt engines, but a checkout has to build its own. This
# installs rustup when absent, pins the toolchain CI uses (the channel in
# rerail/rust-toolchain.toml; RERAIL_RUST_TOOLCHAIN overrides it), and runs the build.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN_FILE="$REPO_ROOT/rerail/rust-toolchain.toml"
pinned_toolchain="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "$TOOLCHAIN_FILE")"
[ -n "$pinned_toolchain" ] || { printf '\033[31m!!\033[0m %s\n' "No channel pinned in $TOOLCHAIN_FILE" >&2; exit 1; }
RERAIL_RUST_TOOLCHAIN="${RERAIL_RUST_TOOLCHAIN:-$pinned_toolchain}"

info() { printf '\033[34m==>\033[0m %s\n' "$*"; }

export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v rustup >/dev/null 2>&1; then
  info "rustup not found; installing to ~/.cargo (no default toolchain, PATH left alone)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain none
fi

info "Ensuring Rust $RERAIL_RUST_TOOLCHAIN"
rustup toolchain install "$RERAIL_RUST_TOOLCHAIN" --profile minimal --component rustfmt --component clippy
# A fresh rustup has no default, so a bare `cargo` (which `npm run build` invokes) fails; give it one.
if ! rustup default >/dev/null 2>&1; then
  info "Setting $RERAIL_RUST_TOOLCHAIN as the default toolchain"
  rustup default "$RERAIL_RUST_TOOLCHAIN"
fi
if [ ! -d "$REPO_ROOT/rerail/node_modules" ]; then
  info "Installing Rerail dependencies"
  npm ci --prefix "$REPO_ROOT/rerail"
fi

info "Building native kernel"
RUSTUP_TOOLCHAIN="$RERAIL_RUST_TOOLCHAIN" npm --prefix "$REPO_ROOT/rerail" run build:native

ls -1 "$REPO_ROOT"/rerail/native/*.node
info "Done. Add \$HOME/.cargo/bin to PATH for \`npm run build\`; restart a running daemon (rerail daemon stop && rerail daemon start) so it loads the kernel."
