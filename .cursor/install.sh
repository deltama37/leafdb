#!/usr/bin/env bash
# Idempotent environment bootstrap for leafdb Cloud Agents.
#
# Ensures the Rust toolchain, the wasm32 target, and wasm-pack are available,
# then builds the native workspace and the WebAssembly package so the demo in
# web/ is ready to serve.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Make cargo-installed binaries visible if a fresh rustup was just installed.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
  echo "==> Installing Rust toolchain"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1091
  source "${CARGO_HOME:-$HOME/.cargo}/env"
fi

echo "==> Ensuring wasm32-unknown-unknown target"
rustup target add wasm32-unknown-unknown

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "==> Installing wasm-pack"
  curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
fi

echo "==> Building native workspace"
cargo build --workspace

echo "==> Building WebAssembly package (web/pkg)"
wasm-pack build crates/leafdb-wasm \
  --target web \
  --out-dir "$REPO_ROOT/web/pkg" \
  --out-name leafdb

echo "==> leafdb environment ready"
