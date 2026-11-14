# Bàn giao phiên làm việc — 2026-11-14

> **Trạng thái:** feature set provider + skills + MCP + plugin + desktop UI đã nằm trên `main`.
> **Commit chính:** `8b474bb` — `feat: add provider book, skills, MCP, plugins, and new desktop UI`.

---

## 1. Đã hoàn thành

### Core

- **Provider book** (`crates/tez-auth/src/providers.rs`)
  - Nhiều provider, mỗi provider một key mã hóa AES-256-GCM.
  - `providers.json` giữ danh sách + `active`; `connection.json` vẫn được đồng bộ để tương thích CLI cũ.
  - Sửa provider mà để trống key sẽ giữ key cũ; xóa provider active sẽ fallback provider còn lại.
- **Skills** (`crates/tez-core/src/skills.rs`)
  - `SKILL.md` + frontmatter: `name`, `description`, `subjects`, `triggers`, `id`.
  - 4 skill mặc định: `step-by-step`, `error-diagnosis`, `exam-review`, `english-writing`.
  - Khớp môn + từ khóa, chèn tối đa 2 skill/lượt vào system prompt.
  - Nạp thêm skill từ plugin qua `load_with_plugins`.
- **MCP** (`crates/tez-core/src/mcp.rs`)
  - Client stdio JSON-RPC: `initialize`, `tools/list`, `tools/call`.
  - Tool name dạng `mcp__<server>__<tool>`.
  - Hỗ trợ `command`, `args`, `env`, `cwd`, `enabled`.
  - Server spawn mỗi lượt chat, timeout 20 giây, tắt khi xong.
- **Plugin** (`crates/tez-core/src/plugins.rs`)
  - `plugin.json` gói skills + MCP server, không chạy code plugin tùy ý.
  - Plugin mẫu: `examples/plugins/study-toolkit/` (skill phòng thi + MCP `echo`/`add`).

### Desktop + CLI

- Desktop viết lại theo hướng editorial study-desk: giấy ấm, xanh rừng + đất nung, serif heading, icon SVG tự vẽ, không gradient/pill/emoji.
- **Cài đặt → Nhà cung cấp:** thêm/sửa/xóa/kiểm tra/chuyển active, preset Groq, Gemini, OpenRouter, OpenAI, Ollama, Custom.
- **Cài đặt → Kỹ năng:** xem môn, từ khóa, nội dung Markdown, mở thư mục.
- **Cài đặt → MCP:** thêm/sửa/xóa server, env, cwd, test, xem tool.
- **Cài đặt → Plugin:** cài plugin mẫu, xem skills/MCP đi kèm.
- **Cài đặt → Dữ liệu:** đường dẫn, số session, mở nhanh thư mục/cấu hình.
- Chat: tool trace có args/output/thời gian, token usage, tải lại session, toast, shortcut `Ctrl N`, `Ctrl ,`.
- CLI: `tez provider`, `tez skills`, `tez mcp`, `tez plugins`; chat inject skills + MCP tools.

### Tài liệu

- `docs/EXTENSIONS.md`: định dạng skill/MCP/plugin + bảo mật.
- `README.md`: cài đặt, tính năng, CLI, desktop, giới hạn.
- `docs/DESKTOP-CLI.md`, `docs/PROVIDER-AUTH.md`: cập nhật trạng thái MVP.

---

## 2. Kiểm chứng

- **56 Rust tests** pass (10 auth + 41 core + 1 MCP integration + 4 store).
- `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `cargo clippy` cho desktop pass.
- `pnpm exec tsc --noEmit` + `pnpm build` pass.
- E2E với mock OpenAI-compatible:
  - thêm + test provider,
  - cài plugin mẫu,
  - test MCP `study` (2 tool),
  - chat gọi `mcp__study__add`, trace trả `5`,
  - DB ghi tool call + session,
  - tải lại session OK.
- Release AppImage build + cài + mở từ menu OK.

**Chưa kiểm chứng:** key provider thật. Toàn bộ E2E dùng mock local tại `127.0.0.1:18080`; chưa gọi Groq/Gemini/OpenRouter thật.

---

## 3. Cài đặt trên máy local

| Thành phần | Vị trí |
|---|---|
| CLI | `~/.local/bin/tez` |
| Desktop | `~/.local/opt/tez/` |
| Desktop entry | `~/.local/share/applications/tez.desktop` |
| Icon | `~/.local/share/icons/hicolor/512x512/apps/tez.png` |
| Dữ liệu | `~/.local/share/teaching-ez/` |

AppImage release từng nằm ở:

```
apps/desktop/src-tauri/target/release/bundle/appimage/tez_0.1.0_amd64.AppImage
```

Sau khi dọn build local, AppImage này sẽ bị xóa; bản đã cài trong `~/.local/opt/tez/` vẫn chạy bình thường.

---

## 4. Chuyển build sang VPS

### Yêu cầu

- Linux x86_64 (cùng kiến trúc với máy local nếu muốn dùng lại AppImage).
- Rust stable (`rustup`), Node 20+, pnpm.
- WebKitGTK/GTK3 dev packages:
  - Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf`
  - Fedora: `webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel patchelf`
- FUSE nếu chạy AppImage trực tiếp; không có FUSE thì dùng `--appimage-extract-and-run`.

### Build

```bash
git clone <repo-url> teaching-ez-harness
cd teaching-ez-harness
./scripts/disk-guard.sh check
./scripts/install-local.sh
```

Hoặc chỉ build desktop:

```bash
./scripts/desktop.sh appimage
```

### Lưu ý

- Build Windows/macOS phải chạy trên OS tương ứng hoặc CI matrix; không cross-compile từ Linux.
- `scripts/disk-guard.sh` có hạn mức cho target, registry, node_modules; chạy `auto` trước build lớn.
- Nếu VPS dùng chung cho nhiều project, đặt `CARGO_TARGET_DIR` riêng và theo dõi quota.
- Không commit `target/`, `node_modules/`, `dist/`; repo đã ignore các thư mục này.

---

## 5. Dọn máy local

Sau khi merge PR close-out:

```bash
cargo clean
(cd apps/desktop/src-tauri && cargo clean)
```

Mục tiêu: xóa toàn bộ Rust build artifacts (`target/`), giữ:

- toolchain Rust,
- CLI + desktop đã cài trong `~/.local/`,
- dữ liệu học tập trong `~/.local/share/teaching-ez/`.

Có thể xóa thêm cache registry nếu thật sự cần:

```bash
rm -rf ~/.cargo/registry/cache ~/.cargo/registry/src
```

Chỉ làm bước này nếu không còn build Rust local; lần build sau sẽ tải lại dependency.

---

## 6. Việc tiếp theo

1. **Test provider thật** (Groq/Gemini/OpenRouter), xác nhận model preset còn hiệu lực; cập nhật default nếu cần.
2. **MCP lỗi không được chặn chat:** hiện `mcp.json` sai cú pháp có thể làm chat/settings fail; nên degrade gracefully và hiển thị cảnh báo.
3. **Giữ tool trace khi tải lại session:** hiện `load_session` chỉ trả message, chưa trả tool call.
4. **Streaming SSE** cho desktop.
5. **RAG PDF/DOCX + OCR**, `context inspect`.
6. **OS keychain** thay `master.key`.
7. **CI + branch protection:** từ phiên sau, làm trên branch + PR, không push trực tiếp `main`; thêm workflow test/clippy/build.

---

## 7. Quy ước phiên sau

- Tạo branch `feat/<mô-tả>` hoặc `fix/<mô-tả>`.
- Chạy `scripts/test.sh` + `scripts/desktop.sh check` trước khi mở PR.
- PR mô tả rõ: thay đổi, cách kiểm chứng, ảnh hưởng dữ liệu/người dùng.
- Chỉ merge khi test/clippy/build pass; không merge khi còn lỗi chưa xử lý.
