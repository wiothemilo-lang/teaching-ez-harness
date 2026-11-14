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
           │ HTTP/SSE 127.0.0.1           │
           └──────────────┬───────────────┘
                          ▼
           ┌─────────────────────────────────┐
           │ Local server (axum)             │
           │  /v1/agent/chat /v1/connections │
           └──────────────┬──────────────────┘
                          ▼
           ┌─────────────────────────────────┐
           │ tez-core (Rust)                 │
           │  agent loop · context · tools   │
           │  pedagogy · rag · domain        │
           └──────────────┬──────────────────┘
                          ▼
        ┌─────────────────┴─────────────────┐
        │ SQLite + FTS5        OS keychain  │
        │ (lessons, traces)    (master key) │
        └─────────────────┬─────────────────┘
                          ▼
              Provider APIs / Ollama
```

**Process model:**

> **MVP hiện tại:** desktop gọi Tauri commands in-process (`app_status`, `chat`, `list_sessions`, `load_session`, `list_providers`, `save_provider`, `delete_provider`, `activate_provider`, `test_provider`, `list_extensions`, `save_mcp_server`, `delete_mcp_server`, `test_mcp_server`, `install_sample_plugin`, `open_extension_path`), dùng chung crate và DB với CLI; chưa có `tez-server`. Khi cần streaming/SSE hoặc nhiều client, sẽ chuyển sang local server như mô tả dưới đây.

- Cài đặt provider trong desktop dùng chung `tez_auth::providers` với CLI: cùng `providers.json`, cùng vault, cùng provider active; `connection.json` được đồng bộ để tương thích bản cũ.
- Lịch sử phiên đọc trực tiếp từ SQLite; bấm một phiên để tải lại message.
- Skills, MCP server và plugin đọc từ `~/.local/share/teaching-ez/`; chat inject skill khớp môn/từ khóa và đăng ký tool MCP vào cùng registry với tool built-in.
- Đóng gói AppImage bằng `scripts/desktop.sh appimage` (release) hoặc `appimage-debug` (nhanh).
- Local server là chủ sở hữu duy nhất của DB, vault, provider calls.
- Tauri shell khởi động server in-process (tokio) và inject `{ url, token }` cho frontend.
- CLI tìm server qua `server.json`; nếu chưa có, chạy one-shot hoặc `tez serve`.
- Server bind `127.0.0.1`, port ngẫu nhiên; token bearer; file `0600`.
- Một server tránh tranh chấp SQLite; streaming/refresh/retry viết một lần.

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
| Local server | **axum + tokio + tower-http** | SSE/streaming tốt, hệ sinh thái mạnh. |
| HTTP client | **reqwest + rustls + eventsource-stream** | Provider streaming. |
| DB | **SQLite + rusqlite (bundled) + refinery** | Local, zero cost, migration. |
| Vector | **FTS5 + embedding BLOB + cosine** (sqlite-vec sau) | Đơn giản cho <10k chunk. |
| Serialize | **serde + serde_json + schemars + jsonschema** | Tool schema/validation. |
| Credential | **keyring + aes-gcm + argon2** | OS keychain + mã hóa. |
| CLI | **clap + dialoguer/inquire + indicatif** | Nhanh; ratatui TUI sau. |
| Tokenizer | **tiktoken-rs** (xấp xỉ cho provider khác) | Context budget. |
| Logging | **tracing + tracing-subscriber** | Debug có cấu trúc. |
| Errors | **thiserror (lib) + anyhow (CLI)** | Rõ ràng. |
| Config | **directories + figment/toml** | Chuẩn OS. |
| Test | **cargo test + wiremock + assert_cmd + insta** | Unit/integration/snapshot. |
| CI | **GitHub Actions** | Build matrix 3 OS. |

**Tauri thay vì Electron:** bạn thích Rust; binary nhỏ; backend Rust tự nhiên. Đổi lại: compile lâu hơn, cần cài system deps (Linux: webkit2gtk, libappindicator...), và học Tauri.

**Không dùng Next.js:** desktop không cần SSR; Vite + React nhẹ hơn.

---

## 5. Local server API

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
tez init
tez connect
tez connect groq --method key
tez connect ollama
tez connect custom
tez auth list
tez auth switch groq work
tez auth logout groq work
tez models
tez chat --subject math --lesson <id>
tez chat --no-tools
tez chat --mode code --workspace ~/du-an
tez update
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
tez context inspect --session <id>
tez trace <session-id>
tez lesson list
tez lesson create
tez lesson ingest <file>
tez subject list
tez usage
tez doctor
tez serve
```

- `--json` cho automation.
- Exit code: 0 ok, 2 auth, 3 quota, 4 tool error.
- Streaming text + trạng thái tool.
- `tez doctor`: server, DB, keychain, provider, tokenizer, disk.

---

## 7. Desktop app

| Màn hình | Chức năng |
|---|---|
| Onboarding | Chọn provider, connect, chọn model |
| **Chat** | Trợ lý tổng quát (không bắt chọn môn), preset môn/lớp tùy chọn, tool trace, token usage |
| **Code** | Workspace chip, `read_file`/`write_file`/`list_files`/`run_command`, tool trace |
| Lessons | Tạo/sửa bài, objectives, tài liệu, ingest |
| Tools | Danh sách tool theo môn, bật/tắt, test |
| Context | Xem section/token/bị cắt |
| Settings → Providers | Nhiều provider, key riêng, test, active, sửa, xóa |
| Settings → Skills | Danh sách skill, môn, từ khóa, nội dung Markdown |
| Settings → MCP | Server stdio, env, cwd, test, danh sách tool |
| Settings → Plugins | Plugin local, skills/MCP đi kèm, cài plugin mẫu |
| Settings → Update | Kiểm tra GitHub Releases, ghi chú, tải asset, mở installer |
| Settings → Data | Đường dẫn dữ liệu, số session, mở thư mục |
| Settings → Models | Model mặc định theo task |
| Usage | Token/latency/chi phí ước tính |
| About/Doctor | Version, log, health |

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
- Migration bằng `refinery` khi server khởi động; backup trước migration lớn.
- Bảng chính: `profiles`, `provider_catalog`, `connections`, `model_preferences`, `courses`, `lessons`, `objectives`, `documents`, `chunks`, `sessions`, `messages`, `agent_steps`, `tool_calls`, `context_snapshots`, `evidence`, `ai_usage`, `settings`.
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
| Local server bị gọi trái phép | 127.0.0.1 + bearer token + Origin check; file token `0600`. |
| Key rò rỉ | keychain + AES-GCM; redaction; không log header. |
| Key lộ qua renderer | Renderer không nhận secret. |
| Tool nguy hiểm | Không shell/code runner MVP; permission + allowlist. |
| Prompt injection | Tài liệu/tool result là data; delimiter; không thực thi. |
| Ghi dữ liệu sai | Write tool confirmation; audit. |
| Update giả | GitHub Releases + checksum; signing sau. |
| Dữ liệu trẻ em | Local-only; age mode; không telemetry mặc định. |

---

## 13. Testing

- Unit: vault, tool math/chemistry, context budget, policy.
- Integration: mock provider + agent loop; SSE.
- CLI e2e: `assert_cmd`; connect/chat/tools/context.
- Desktop smoke: Tauri driver/Playwright (sau).
- Migration: schema cũ → mới, không mất dữ liệu.
- Security: secret không có trong log/response; token sai 401.

---

## 14. Lộ trình

| Phase | Thời lượng | Kết quả |
|---|---|---|
| 0. Rust basics | 1–2 tuần | CLI nhỏ, hiểu ownership/async |
| 1. Core + CLI | 2–3 tuần | `tez chat` streaming |
| 2. Auth + store | 2 tuần | `tez connect/models`, vault, SQLite |
| 3. Agent loop | 2–3 tuần | Context + tools + 6 tool chung |
| 4. Subject packs | 2–3 tuần | 4 môn mỏng, RAG, evidence, eval |
| 5. Tauri | 2–3 tuần | Desktop UI, tool cards, context inspector |
| 6. Packaging | 1–2 tuần | Installer 3 OS, beta |

**Tổng:** 12–18 tuần part-time; 5h/tuần thì 6–8 tháng.

---

## 15. Đánh đổi

| Quyết định | Chọn | Lý do |
|---|---|---|
| Desktop shell | Tauri | Rust, binary nhỏ |
| Local server | axum | Share desktop/CLI, SSE |
| DB | SQLite | Local, zero cost |
| Provider | OpenAI-compatible trước | Phủ Groq/OpenRouter/Cloudflare/Ollama |
| Agent | Tự viết loop | Kiểm soát pedagogy/tools/context |
| Vector | FTS5 + cosine | Đơn giản; sqlite-vec sau |
| CLI | clap + dialoguer | Nhanh; ratatui sau |

---

## 16. Câu hỏi còn mở

1. CAS Toán: numeric/step-check hay SymPy sidecar?
2. Grammar Anh: `harper` hay model?
3. Tool write: auto hay confirmation?
4. Subject pack: compile-time hay load file?
5. Lesson pack trong MVP?
6. Tauri updater bật ngay?

---

*Tài liệu liên quan: [PLAN.md](PLAN.md) · [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md) · [PROVIDER-AUTH.md](PROVIDER-AUTH.md)*
