# Desktop app + CLI — Tauri, Rust, local-first

> **Trạng thái:** Draft v0.2 (thay thế bản Electron)
> **Quyết định:** Tauri 2 + Rust core, CLI `tez`, local-first, BYOK.
> **Tài liệu agent/tools/context:** [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md).

---

## 1. Vì sao local-first

| Lợi ích | Ý nghĩa |
|---|---|
| **0 đồng hosting** | Không server, không DB cloud. Chỉ GitHub Releases. |
| **Key không rời máy** | API key/OAuth token trong OS keychain + SQLite mã hóa. |
| **Offline** | Ollama/LM Studio khi không có mạng; tài liệu local. |
| **Phù hợp học sinh/phụ huynh** | Không thu thập dữ liệu trẻ em trên cloud. |
| **Desktop + CLI share core** | Cùng Rust core, cùng DB, cùng connection. |
| **Rust** | Binary nhỏ, nhanh, an toàn bộ nhớ; kỹ năng có giá trị lâu dài. |

**Đánh đổi:** thời gian học Rust lâu hơn Electron; phải đóng gói installer từng OS; chưa có sync đa máy.

---

## 2. Kiến trúc

```
┌─────────────────────┐        ┌─────────────────────┐
│ Tauri desktop       │        │ CLI (tez)           │
│ React + Vite        │        │ clap + dialoguer    │
└──────────┬──────────┘        └──────────┬──────────┘
           │ Tauri commands               │ gọi trực tiếp
           └──────────────┬───────────────┘
                          ▼
           ┌─────────────────────────────────┐
           │ tez-core (Rust)                 │
           │  agent loop · harness · context │
           │  tools · pedagogy · rag         │
           └──────────────┬──────────────────┘
                          ▼
        ┌─────────────────┴─────────────────┐
        │ SQLite + FTS5        OS keychain  │
        │ (sessions, todos)    (master key) │
        └─────────────────┬─────────────────┘
                          ▼
              Provider APIs / Ollama
```

**Process model:**

> **MVP hiện tại:** desktop gọi Tauri commands in-process, dùng chung crate và DB với CLI; chưa có `tez-server`. Nhóm command chính:
>
> - Chat/session: `app_status`, `chat`, `list_sessions`, `load_session`.
> - Harness: `respond_approval`, `get_harness`, `save_harness`, `delete_memory`.
> - Provider: `list_providers`, `save_provider`, `delete_provider`, `activate_provider`, `test_provider`, `test_connection`.
> - Extension: `list_extensions`, `save_mcp_server`, `delete_mcp_server`, `test_mcp_server`, `install_sample_plugin`, `open_extension_path`.
> - Update/file: `check_update`, `download_update`, `launch_update`, `open_external`, `open_workspace`.
>
> Khi một tool cần duyệt, `DesktopApprover` phát event `tez://approval-request`; React mở modal rồi gọi `respond_approval({ id, approved })`. Khi cần streaming/SSE hoặc nhiều client, sẽ chuyển sang local server như mô tả dưới đây.

- Cài đặt provider trong desktop dùng chung `tez_auth::providers` với CLI: cùng `providers.json`, cùng vault, cùng provider active; `connection.json` được đồng bộ để tương thích bản cũ.
- Lịch sử phiên đọc trực tiếp từ SQLite; bấm một phiên để tải lại message.
- Skills, MCP server và plugin đọc từ `~/.local/share/teaching-ez/`; chat inject skill khớp môn/từ khóa và đăng ký tool MCP vào cùng registry với tool built-in.
- Đóng gói AppImage bằng `scripts/desktop.sh appimage` (release) hoặc `appimage-debug` (nhanh).
- Desktop và CLI đều mở SQLite trực tiếp; WAL + `busy_timeout` xử lý tranh chấp ngắn. Không có process server riêng.
- Mỗi lệnh chat mở Store, nạp session/todo, chạy agent rồi ghi message/tool call/usage; lỗi DB trả về UI dưới dạng chuỗi thân thiện.
- Provider call nằm trong `tez-core` qua trait `LlmProvider`; vault/keychain nằm trong `tez-auth`.
- Streaming/SSE và local server vẫn là kế hoạch khi cần nhiều client; hiện tại token trả một lần.

---

## 3. Workspace

```
Cargo.toml
crates/
  tez-core/       # agent, context, tools, pedagogy, domain
  tez-llm/        # provider trait + OpenAI-compatible + streaming
  tez-auth/       # keychain, vault, connections, OAuth
  tez-store/      # SQLite, migrations, repos
  tez-server/     # axum + SSE
  tez-cli/        # binary `tez`
apps/
  desktop/        # React + Vite
  desktop/src-tauri/
subjects/
  math/ physics/ chemistry/ english/
```

---

## 4. Stack

| Lớp | Chọn | Vì sao |
|---|---|---|
| Ngôn ngữ | **Rust** | Core/CLI/desktop backend một ngôn ngữ; an toàn, nhanh. |
| Desktop shell | **Tauri 2** | Binary nhỏ, dùng webview hệ thống; Rust backend. |
| Frontend | **React + Vite + CSS design system** | Không cần SSR; build nhanh; giao diện editorial tự viết, không phụ thuộc UI kit. |
| Local server | **chưa triển khai** | Hiện Tauri commands in-process; thêm axum/SSE khi cần nhiều client. |
| HTTP client | **reqwest + rustls** | Gọi provider OpenAI-compatible. |
| DB | **SQLite + rusqlite (bundled)** | Local, zero cost; migration SQL idempotent trong `tez-store`. |
| Vector | **FTS5 + embedding BLOB + cosine** (sqlite-vec sau) | Đơn giản cho <10k chunk. |
| Serialize | **serde + serde_json** | Tool schema/JSON; chưa dùng schemars/jsonschema. |
| Credential | **keyring + aes-gcm + argon2** | OS keychain + mã hóa. |
| CLI | **clap + dialoguer** | Nhanh; ratatui TUI sau. |
| Tokenizer | **chưa dùng** | Ước lượng context bằng độ dài JSON; thêm `tiktoken-rs` khi cần. |
| Logging | **tracing + tracing-subscriber** | Debug có cấu trúc. |
| Errors | **thiserror (lib) + anyhow (CLI)** | Rõ ràng. |
| Config | **directories + serde_json** | Chuẩn OS; `harness.json`, `providers.json`. |
| Test | **cargo test + mock provider** | Unit/integration; `assert_cmd` cho CLI e2e sau. |
| CI | **GitHub Actions** | Build matrix 3 OS. |

**Tauri thay vì Electron:** bạn thích Rust; binary nhỏ; backend Rust tự nhiên. Đổi lại: compile lâu hơn, cần cài system deps (Linux: webkit2gtk, libappindicator...), và học Tauri.

**Không dùng Next.js:** desktop không cần SSR; Vite + React nhẹ hơn.

---

## 5. Local server API (kế hoạch)

```
GET  /health
GET  /v1/providers
GET  /v1/models
GET  /v1/connections
POST /v1/connections
POST /v1/connections/:id/test
DELETE /v1/connections/:id

GET  /v1/subjects
GET  /v1/tools
POST /v1/agent/chat          # SSE
GET  /v1/sessions
GET  /v1/sessions/:id
GET  /v1/sessions/:id/trace
POST /v1/context/inspect
GET  /v1/usage
```

- Auth: bearer token từ `server.json`.
- SSE events: `delta`, `tool_call`, `tool_result`, `state`, `usage`, `done`, `error`.
- Không trả secret; connection chỉ trả metadata + masked.

---

## 6. CLI `tez`

```bash
tez connect groq --method key
tez connect ollama
tez connect custom
tez chat "Giải thích borrow checker"
tez chat --subject math "Giải phương trình 2x + 3 = 7"
tez chat --mode code --workspace ~/du-an "Sửa lỗi build"
tez chat --session <id> --history 20 "Tiếp tục việc còn dở"
tez chat --yes --permission trust --max-steps 20 "Chạy test"
tez chat --permission read_only "Đọc và giải thích kiến trúc"
tez chat --no-memory "Bỏ qua memory đã lưu"
tez update --check-only
tez update --download
tez tools list
tez tools test calculate --args '{"expression":"2+2"}'
tez provider list
tez provider use <id>
tez skills list
tez skills show <id>
tez mcp list
tez mcp test [server]
tez plugins
tez subject list
tez sessions
tez memory list
tez memory search "quy ước build"
tez memory forget "Build command"
tez harness
tez usage
tez doctor
```

- `tez chat` không có prompt và không có stdin pipe thì vào chế độ chat
  tương tác; có pipe thì đọc một lần từ stdin.
- `--mode code` dùng workspace tools; `--workspace` mặc định là thư mục hiện
  tại. `--session` khôi phục mode/môn học đã lưu.
- `--yes` tự duyệt mọi thao tác cần permission; `--permission` ghi đè
  `harness.json`; `--max-steps` và `--no-memory` ghi đè cho riêng lượt chạy.
- `tez memory list|search|forget` và `tez harness` xem bộ nhớ dài hạn và cấu
  hình harness.
- Output hiện tại là văn bản cuối cùng; tool trace, cảnh báo permission và
  thống kê nén đi ra stderr. Chưa có streaming token.
- Lỗi trả exit code 1 kèm thông báo. `tez doctor` kiểm tra data dir, provider,
  DB, skill, MCP, plugin, harness và dung lượng.

---

## 7. Desktop app

| Màn hình | Chức năng |
|---|---|
| **Chat** | Trợ lý tổng quát (không bắt chọn môn), preset môn/lớp tùy chọn, tool trace, token usage, plan/context dock |
| **Code** | Workspace chip, `read_file`/`write_file`/`edit_file`/`list_files`/`glob`/`grep`/`run_command`, todo/context dock, approval modal |
| **Approval modal** | Hiện lý do + tool + args JSON; từ chối hoặc cho phép; timeout 180 s thì từ chối |
| **Harness dock** | Tiến độ todo, context meter, số lượt nén, số thao tác bị chặn, nút mở Settings → Harness |
| Settings → Providers | Nhiều provider, key riêng, test, active, sửa, xóa |
| Settings → Skills | Danh sách skill, môn, từ khóa, nội dung Markdown |
| Settings → MCP | Server stdio, env, cwd, test, danh sách tool |
| Settings → Plugins | Plugin local, skills/MCP đi kèm, cài plugin mẫu |
| **Settings → Harness** | Permission mode, giới hạn agent, compaction, memory + xoá bản ghi, hook shell, đường dẫn config/audit |
| Settings → Update | Kiểm tra GitHub Releases, ghi chú, tải asset, mở installer |
| Settings → Data | Đường dẫn dữ liệu, số session, mở thư mục |
| Usage | Token/latency/chi phí ước tính *(kế hoạch)* |
| About/Doctor | Version, log, health *(kế hoạch)* |

Các màn Lessons, Tools, Context inspector và Models theo mô tả cũ vẫn là kế
hoạch; bản hiện tại tập trung vào Chat/Code, extension và harness.

**Tauri security:**

- Capabilities allowlist: dialog, opener, updater, core.
- CSP chặt; không load remote content.
- `shell.openExternal` chỉ URL allowlist (OAuth).
- Frontend không nhận secret.

---

## 8. Credential vault (Rust)

1. Master key 32 byte ngẫu nhiên lưu trong **OS keychain** qua crate `keyring`.
   - macOS Keychain, Windows Credential Manager, Linux secret-service.
2. SQLite lưu ciphertext AES-256-GCM (`aes-gcm` crate).
3. Chỉ local server decrypt; desktop/CLI không nhận lại secret.
4. Fallback nếu keychain không có: passphrase → `argon2` → key; hoặc file `0600` + cảnh báo.

```rust
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::{Aead, OsRng, rand_core::RngCore}};

pub fn encrypt(key: &[u8; 32], plain: &[u8]) -> anyhow::Result<(Vec<u8>, [u8; 12])> {
    let cipher = Aes256Gcm::new(key.into());
    let mut iv = [0u8; 12];
    OsRng.fill_bytes(&mut iv);
    let ct = cipher.encrypt(Nonce::from_slice(&iv), plain)
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    Ok((ct, iv))
}
```

Chi tiết: [PROVIDER-AUTH.md](PROVIDER-AUTH.md).

---

## 9. SQLite

- `PRAGMA journal_mode=WAL`, `foreign_keys=ON`.
- Migration hiện tại là schema version 5; `Store::migrate` chạy `SCHEMA` idempotent rồi thêm cột còn thiếu (`sessions.subject`, `sessions.mode`).
- Bảng chính: `profiles`, `connections`, `lessons`, `sessions`, `messages`, `context_snapshots`, `tool_calls`, `session_todos`, `ai_usage`, `documents`, `document_chunks`, `document_chunks_fts`, `schema_migrations`.
- `session_todos` giữ kế hoạch của phiên dưới dạng JSON array, khoá theo `session_id`; desktop/CLI nạp lại khi resume.
- Không multi-tenant; mỗi máy một DB.

Chi tiết schema: [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md#8-storage).

---

## 10. RAG local

1. Copy file vào `~/.local/share/teaching-ez/documents/<id>/`.
2. Trích text: PDF (`pdf-extract`/`lopdf`), DOCX (`docx-rs` hoặc unzip XML), TXT/MD.
3. Chunk 500–800 token, overlap 100.
4. Embed: Ollama `nomic-embed-text` (offline) hoặc Gemini/Cloudflare.
5. Lưu BLOB Float32Array.
6. Retrieve: FTS5 + cosine; top-k; citation `{document, page, snippet}`.

**Prompt injection:** tài liệu là data; delimiter; không thực thi chỉ dẫn; tool allowlist.

**Lesson pack `.tezlesson`:** JSON + tài liệu + prompt/strategy; import/export offline; embedding tạo lại khi import.

---

## 11. Đóng gói & phát hành

### Desktop

- `tauri build`:
  - Linux: deb, rpm, AppImage
  - macOS: dmg (arm64 + x64)
  - Windows: msi/nsis
- GitHub Actions matrix: ubuntu/macos/windows; cache cargo; cài system deps Linux.
- Chưa code signing: hướng dẫn mở installer unsigned; code signing khi có điều kiện.
- Auto-update: Tauri updater + GitHub Releases (sau stable).

### CLI

- `cargo build --release -p tez-cli`.
- Phát hành binary qua GitHub Releases; hoặc `cargo install --path crates/tez-cli`.
- `cargo dist` (tùy chọn) để build đa nền tảng.
- `tez --version`, `tez doctor`.

---

## 12. Bảo mật

| Rủi ro | Biện pháp |
|---|---|
| Local server (tương lai) | Nếu bật `tez-server`: 127.0.0.1 + bearer token + Origin check; file token `0600`. Hiện desktop gọi Tauri command in-process. |
| Key rò rỉ | keychain + AES-GCM; redaction; không log header. |
| Key lộ qua renderer | Renderer không nhận secret. |
| Tool nguy hiểm | Code mode có `run_command` nhưng bị giới hạn trong workspace ở tầng ứng dụng, qua permission pipeline (deny → ask → allow) và audit/redaction; không phải sandbox OS. |
| Prompt injection | Tài liệu/tool result là data; delimiter; không thực thi chỉ dẫn; tool allowlist. |
| Ghi dữ liệu sai | Write/edit tool qua permission; audit JSONL; todo/plan hiển thị để người dùng theo dõi. |
| Update giả | GitHub Releases + checksum; signing sau. |
| Dữ liệu trẻ em | Local-only; age mode; không telemetry mặc định. |

---

## 13. Testing

- Unit: vault, tool math/chemistry, permission pipeline (deny → ask → allow, bốn mode), compaction 4 bước, memory save/search/forget, todo validation, hook block/redaction, harness sanitize, store migration v5 và `session_todos`.
- Integration: mock provider + agent loop; permission denial; subagent; MCP stdio; artifact compaction (`sessions/<id>/tool-results|transcripts`).
- CLI e2e: `assert_cmd`; connect/chat/tools/context; `--yes`/`--permission`/`--max-steps`; `memory`, `harness`.
- Desktop smoke: Tauri driver/Playwright (sau); preview mock hiện kiểm tra approval modal, Harness dock và Settings → Harness.
- Frontend: `tsc --noEmit` + Vite build sạch.
- Migration: schema cũ → v5, không mất dữ liệu; todo của phiên cũ được giữ.
- Security: secret không có trong log/response; audit JSONL; redaction; token sai 401 (khi có server).

---

## 14. Lộ trình

| Phase | Thời lượng | Kết quả |
|---|---|---|
| 0. Rust basics | 1–2 tuần | CLI nhỏ, hiểu ownership/async |
| 1. Core + CLI | 2–3 tuần | `tez chat` (hiện non-streaming) |
| 2. Auth + store | 2 tuần | `tez connect/models`, vault, SQLite |
| 3. Agent loop + harness | 2–3 tuần | Context + tools + permission/hooks/todo/subagent/compaction/memory |
| 4. Subject packs | 2–3 tuần | 4 môn mỏng, RAG, evidence, eval |
| 5. Tauri | 2–3 tuần | Desktop UI, approval modal, Harness dock, context inspector |
| 6. Packaging | 1–2 tuần | Installer 3 OS, updater, beta |

**Tổng:** 12–18 tuần part-time; 5h/tuần thì 6–8 tháng.

---

## 15. Đánh đổi

| Quyết định | Chọn | Lý do |
|---|---|---|
| Desktop shell | Tauri | Rust, binary nhỏ |
| Desktop ↔ core | Tauri command in-process | Không cần server/SSE; ít bề mặt tấn công |
| DB | SQLite | Local, zero cost |
| Provider | OpenAI-compatible trước | Phủ Groq/OpenRouter/Cloudflare/Ollama |
| Agent | Tự viết loop | Kiểm soát pedagogy/tools/context |
| Vector | FTS5 + cosine | Đơn giản; sqlite-vec sau |
| CLI | clap + dialoguer | Nhanh; ratatui sau |

---

## 16. Câu hỏi còn mở

1. CAS Toán: numeric/step-check hay SymPy sidecar?
2. Grammar Anh: `harper` hay model?
3. Memory: giữ keyword scoring hay thêm embedding local?
4. Subject pack: compile-time hay load file?
5. Lesson pack trong MVP?
6. Code mode: có cần sandbox OS (container/landlock) ngoài giới hạn workspace?

---

*Tài liệu liên quan: [PLAN.md](PLAN.md) · [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md) · [PROVIDER-AUTH.md](PROVIDER-AUTH.md)*
