#!/usr/bin/env bash
# Cài tez cho user hiện tại: CLI + desktop AppImage + menu entry.
# Chỉ ghi vào ~/.local, không cần sudo.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/opt/tez"
APPLICATIONS_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/512x512/apps"

export PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${PATH}"
if command -v fnm >/dev/null 2>&1; then
  eval "$(fnm env)"
fi

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Thiếu '$1'. Cài trước rồi chạy lại." >&2
    exit 1
  fi
}

need cargo
need pnpm

echo "→ Build CLI release"
(cd "$ROOT" && cargo build --release -p tez-cli --locked)

echo "→ Build desktop AppImage (release)"
(
  cd "$ROOT/apps/desktop"
  pnpm install --frozen-lockfile
  pnpm tauri build --bundles appimage
)

APPIMAGE="$(find "$ROOT/apps/desktop/src-tauri/target/release/bundle/appimage" -maxdepth 1 -name '*.AppImage' -print -quit)"
if [ -z "$APPIMAGE" ]; then
  echo "Không tìm thấy AppImage sau khi build." >&2
  exit 1
fi

echo "→ Cài CLI: $BIN_DIR/tez"
install -Dm755 "$ROOT/target/release/tez" "$BIN_DIR/tez"

echo "→ Cài desktop: $APP_DIR"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
(
  cd "$TMP_DIR"
  "$APPIMAGE" --appimage-extract >/dev/null
)
rm -rf "$APP_DIR"
mkdir -p "$(dirname "$APP_DIR")"
mv "$TMP_DIR/squashfs-root" "$APP_DIR"

ICON_SRC="$APP_DIR/tez.png"
if [ ! -f "$ICON_SRC" ]; then
  ICON_SRC="$ROOT/apps/desktop/src-tauri/icons/icon.png"
fi
install -Dm644 "$ICON_SRC" "$ICON_DIR/tez.png"

cat > "$APPLICATIONS_DIR/tez.desktop" <<EOF
[Desktop Entry]
Name=tez — Gia sư AI
Comment=Gia sư AI local-first cho học sinh
Exec=${APP_DIR}/AppRun
Icon=tez
Terminal=false
Type=Application
Categories=Education;
StartupWMClass=tez-desktop
EOF

update-desktop-database "$APPLICATIONS_DIR" >/dev/null 2>&1 || true
gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true

echo
echo "Xong."
echo "  CLI:     tez doctor"
echo "  Desktop: tìm 'tez' trong menu, hoặc chạy: $APP_DIR/AppRun"
