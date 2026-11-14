#!/usr/bin/env bash
# build.sh — chạy disk guard trước khi build để tránh đầy ổ.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT/scripts/disk-guard.sh" auto

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
cd "$ROOT"
cargo build "$@"
