#!/usr/bin/env bash
# desktop.sh — build/check/chạy Tauri desktop.
# Dùng: scripts/desktop.sh dev|check|build
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT/scripts/disk-guard.sh" auto

export PATH="$HOME/.local/bin:${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
if command -v fnm >/dev/null 2>&1; then
  eval "$(fnm env)"
fi

cd "$ROOT/apps/desktop"

usage() {
  cat <<EOF
Dùng: scripts/desktop.sh <dev|check|build|appimage|appimage-debug>

  dev             Chạy Vite + Tauri ở chế độ phát triển
  check           Build frontend + cargo check/clippy cho Tauri
  build           Build frontend + binary Tauri + installer (theo tauri.conf.json)
  appimage        Đóng gói AppImage (release)
  appimage-debug  Đóng gói AppImage bản debug, nhanh hơn
EOF
}

case "${1:-dev}" in
  dev)
    pnpm tauri dev
    ;;
  check)
    pnpm build
    cd src-tauri
    cargo check
    cargo clippy --all-targets -- -D warnings
    ;;
  build)
    pnpm build
    pnpm tauri build
    ;;
  appimage)
    pnpm build
    pnpm tauri build --bundles appimage
    ;;
  appimage-debug)
    pnpm build
    pnpm tauri build --debug --bundles appimage
    ;;
  *)
    usage
    exit 2
    ;;
esac
