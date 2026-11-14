#!/usr/bin/env bash
# test.sh — disk guard + fmt + clippy + test.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT/scripts/disk-guard.sh" auto

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
cd "$ROOT"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Desktop có workspace Cargo riêng; chỉ kiểm tra khi đã cài node_modules.
if [ -f "$ROOT/apps/desktop/src-tauri/Cargo.toml" ] && [ -d "$ROOT/apps/desktop/node_modules" ]; then
  "$ROOT/scripts/desktop.sh" check
fi
