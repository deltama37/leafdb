#!/usr/bin/env bash
# Builds the leafdb WebAssembly package into web/pkg.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

wasm-pack build crates/leafdb-wasm \
  --target web \
  --out-dir "$ROOT/web/pkg" \
  --out-name leafdb

echo "Built web/pkg (wasm: $(du -h "$ROOT/web/pkg/leafdb_bg.wasm" | cut -f1))"
