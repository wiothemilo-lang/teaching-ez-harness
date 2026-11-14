#!/usr/bin/env bash
# disk-guard.sh — kiểm tra và dọn dẹp dung lượng theo hạn mức.
# Dùng: scripts/disk-guard.sh check|clean|auto
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/disk-quotas.env
source "$ROOT/scripts/disk-quotas.env"

RUSTUP_DIR="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_DIR="${CARGO_HOME:-$HOME/.cargo}"
TARGET_DIR="$ROOT/target"
DESKTOP_TARGET="$ROOT/apps/desktop/src-tauri/target"
FNM_DIR="${FNM_DIR:-$HOME/.local/share/fnm}"
PNPM_STORE="${PNPM_HOME:-$HOME/.local/share/pnpm}/store"
APP_DATA="${XDG_DATA_HOME:-$HOME/.local/share}/teaching-ez"
LOG_DIR="$APP_DATA/logs"
OLLAMA_MODELS="${OLLAMA_MODELS:-$HOME/.ollama/models}"

WARN_COUNT=0
OVER_COUNT=0
TOTAL_BYTES=0

to_bytes() {
  local v="$1"
  case "$v" in
    *T) echo $(( ${v%T} * 1024 * 1024 * 1024 * 1024 )) ;;
    *G) echo $(( ${v%G} * 1024 * 1024 * 1024 )) ;;
    *M) echo $(( ${v%M} * 1024 * 1024 )) ;;
    *K) echo $(( ${v%K} * 1024 )) ;;
    *)  echo "$v" ;;
  esac
}

fmt() {
  if command -v numfmt >/dev/null 2>&1; then
    numfmt --to=iec --suffix=B "$1" 2>/dev/null || echo "${1}B"
  else
    echo "${1}B"
  fi
}

dir_size() {
  local p="$1"
  [ -e "$p" ] || { echo 0; return; }
  du -sb "$p" 2>/dev/null | awk '{print $1}' | head -1
}

node_modules_size() {
  find "$ROOT" -type d -name node_modules -prune -print0 2>/dev/null \
    | xargs -0 -r du -sb 2>/dev/null \
    | awk '{s+=$1} END {print s+0}'
}

status_of() {
  local bytes="$1" soft="$2" hard="$3"
  if (( bytes >= hard )); then echo "OVER"
  elif (( bytes >= soft )); then echo "WARN"
  else echo "OK"; fi
}

check_size() {
  local name="$1" bytes="$2" soft="$3" hard="$4"
  local s; s="$(status_of "$bytes" "$(to_bytes "$soft")" "$(to_bytes "$hard")")"
  case "$s" in
    OVER) OVER_COUNT=$((OVER_COUNT + 1)) ;;
    WARN) WARN_COUNT=$((WARN_COUNT + 1)) ;;
  esac
  TOTAL_BYTES=$((TOTAL_BYTES + bytes))
  printf "%-22s %12s %12s %12s  %s\n" "$name" "$(fmt "$bytes")" "$soft" "$hard" "$s"
}

check_all() {
  WARN_COUNT=0
  OVER_COUNT=0
  TOTAL_BYTES=0

  printf "%-22s %12s %12s %12s  %s\n" "Hạng mục" "Đang dùng" "Mềm" "Cứng" "Trạng thái"
  printf '%.0s-' {1..72}; echo

  check_size "Rust toolchain" "$(dir_size "$RUSTUP_DIR")" "$QUOTA_RUSTUP" "$QUOTA_RUSTUP_HARD"
  check_size "Cargo registry" "$(dir_size "$CARGO_DIR/registry")" "$QUOTA_CARGO_REGISTRY" "$QUOTA_CARGO_REGISTRY_HARD"
  check_size "Cargo git" "$(dir_size "$CARGO_DIR/git")" "$QUOTA_CARGO_GIT" "$QUOTA_CARGO_GIT_HARD"
  check_size "Build target/" "$(dir_size "$TARGET_DIR")" "$QUOTA_TARGET" "$QUOTA_TARGET_HARD"
  check_size "Desktop target" "$(dir_size "$DESKTOP_TARGET")" "$QUOTA_DESKTOP_TARGET" "$QUOTA_DESKTOP_TARGET_HARD"
  check_size "Node runtime" "$(dir_size "$FNM_DIR")" "$QUOTA_FNM" "$QUOTA_FNM_HARD"
  check_size "pnpm store" "$(dir_size "$PNPM_STORE")" "$QUOTA_PNPM_STORE" "$QUOTA_PNPM_STORE_HARD"
  check_size "node_modules" "$(node_modules_size)" "$QUOTA_NODE_MODULES" "$QUOTA_NODE_MODULES_HARD"
  check_size "App data" "$(dir_size "$APP_DATA")" "$QUOTA_APP_DATA" "$QUOTA_APP_DATA_HARD"
  check_size "Logs" "$(dir_size "$LOG_DIR")" "$QUOTA_LOGS" "$QUOTA_LOGS_HARD"
  check_size "Ollama models" "$(dir_size "$OLLAMA_MODELS")" "$QUOTA_OLLAMA" "$QUOTA_OLLAMA_HARD"

  printf '%.0s-' {1..72}; echo
  local total_soft total_hard
  total_soft="$(to_bytes "$QUOTA_TOTAL_SOFT")"
  total_hard="$(to_bytes "$QUOTA_TOTAL_HARD")"
  local ts; ts="$(status_of "$TOTAL_BYTES" "$total_soft" "$total_hard")"
  printf "%-22s %12s %12s %12s  %s\n" "TỔNG" "$(fmt "$TOTAL_BYTES")" "$QUOTA_TOTAL_SOFT" "$QUOTA_TOTAL_HARD" "$ts"

  echo
  if (( OVER_COUNT > 0 )); then
    echo "Có $OVER_COUNT hạng mục vượt hạn cứng; chạy: scripts/disk-guard.sh clean"
  elif (( WARN_COUNT > 0 )); then
    echo "Có $WARN_COUNT hạng mục vượt hạn mềm; nên dọn: scripts/disk-guard.sh clean"
  else
    echo "Tất cả trong hạn mức."
  fi
}

clean_cargo() {
  echo "→ Cargo registry: xóa cache > ${RETENTION_CARGO_DAYS} ngày"
  find "$CARGO_DIR/registry/cache" -type f -mtime +"$RETENTION_CARGO_DAYS" -delete 2>/dev/null || true
  find "$CARGO_DIR/git/checkouts" -mindepth 1 -maxdepth 1 -type d \
    -mtime +"$RETENTION_CARGO_DAYS" -exec rm -rf {} + 2>/dev/null || true
}

clean_target() {
  echo "→ Build target: xóa incremental"
  rm -rf "$TARGET_DIR/debug/incremental" "$TARGET_DIR/release/incremental" 2>/dev/null || true
  if [ "$(dir_size "$TARGET_DIR")" -ge "$(to_bytes "$QUOTA_TARGET")" ]; then
    if [ -f "$ROOT/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
      echo "→ Build target vẫn vượt hạn; chạy cargo clean"
      (cd "$ROOT" && cargo clean) || true
    else
      echo "→ Build target vẫn vượt hạn; xóa thư mục target/"
      rm -rf "$TARGET_DIR" 2>/dev/null || true
    fi
  fi
}

clean_desktop_target() {
  echo "→ Desktop target: xóa incremental"
  rm -rf "$DESKTOP_TARGET/debug/incremental" "$DESKTOP_TARGET/release/incremental" 2>/dev/null || true
  if [ "$(dir_size "$DESKTOP_TARGET")" -ge "$(to_bytes "$QUOTA_DESKTOP_TARGET")" ]; then
    if command -v cargo >/dev/null 2>&1; then
      echo "→ Desktop target vẫn vượt hạn; chạy cargo clean"
      (cd "$ROOT/apps/desktop/src-tauri" && cargo clean) || true
    else
      echo "→ Desktop target vẫn vượt hạn; xóa thư mục target/"
      rm -rf "$DESKTOP_TARGET" 2>/dev/null || true
    fi
  fi
}

clean_pnpm() {
  if command -v pnpm >/dev/null 2>&1; then
    echo "→ pnpm store: prune"
    pnpm store prune >/dev/null 2>&1 || true
  else
    echo "→ pnpm chưa có; bỏ qua pnpm store"
  fi
}

clean_node_modules() {
  echo "→ node_modules: xóa cache .cache/.vite"
  find "$ROOT" -type d \( -name .cache -o -name .vite \) -prune \
    -exec rm -rf {} + 2>/dev/null || true
}

clean_app_data() {
  echo "→ App data: log > ${RETENTION_LOG_DAYS} ngày, file log > 50MB"
  if [ -d "$LOG_DIR" ]; then
    find "$LOG_DIR" -type f -mtime +"$RETENTION_LOG_DAYS" -delete 2>/dev/null || true
    find "$LOG_DIR" -type f -size +50M -exec truncate -s 50M {} \; 2>/dev/null || true
  fi

  if command -v sqlite3 >/dev/null 2>&1 && [ -f "$APP_DATA/tez.db" ]; then
    echo "→ SQLite: retention + VACUUM"
    sqlite3 "$APP_DATA/tez.db" <<SQL 2>/dev/null || true
DELETE FROM context_snapshots
 WHERE created_at < datetime('now','-${RETENTION_CONTEXT_DAYS} days');
DELETE FROM sessions
 WHERE ended_at IS NOT NULL
   AND ended_at < datetime('now','-${RETENTION_SESSION_DAYS} days');
PRAGMA wal_checkpoint(TRUNCATE);
VACUUM;
SQL
  fi
}

clean_all() {
  echo "Bắt đầu dọn dẹp an toàn..."
  clean_cargo
  clean_target
  clean_desktop_target
  clean_pnpm
  clean_node_modules
  clean_app_data
  echo "→ Rust toolchain / Node runtime / Ollama: chỉ cảnh báo, không tự xóa."
  echo "Xong."
}

usage() {
  cat <<EOF
Dùng: scripts/disk-guard.sh <check|clean|auto>

  check  Kiểm tra dung lượng và hạn mức
  clean  Dọn cache/build/log/app data vượt hạn mềm
  auto   check + clean nếu cần + check lại; exit 1 nếu tổng vượt cứng
EOF
}

case "${1:-check}" in
  check)
    check_all
    ;;
  clean)
    check_all
    clean_all
    echo
    check_all
    ;;
  auto)
    check_all
    if (( WARN_COUNT > 0 || OVER_COUNT > 0 )); then
      echo
      clean_all
      echo
      check_all
    fi
    total_hard="$(to_bytes "$QUOTA_TOTAL_HARD")"
    if (( TOTAL_BYTES >= total_hard )); then
      echo "TỔNG vẫn vượt hạn cứng; dừng để tránh đầy ổ." >&2
      exit 1
    fi
    ;;
  *)
    usage
    exit 2
    ;;
esac
