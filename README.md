# teaching-ez-harness

[![CodeRabbit Reviews](https://img.shields.io/badge/CodeRabbit-reviewed-FF570A?logo=coderabbit&logoColor=white)](https://coderabbit.ai)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app/)
[![Tests](https://img.shields.io/badge/tests-56%20passing-brightgreen)](#build--test)
[![Version](https://img.shields.io/badge/version-0.1.0-blue)](Cargo.toml)
[![Local-first](https://img.shields.io/badge/local--first-BYOK-2F7D5B)](docs/DESKTOP-CLI.md)
[![Platform](https://img.shields.io/badge/platform-Linux%20%C2%B7%20AppImage-lightgrey?logo=linux&logoColor=black)](#desktop-tauri)
[![License](https://img.shields.io/badge/license-MIT-yellow)](LICENSE)

Harness local-first để biến model AI của bạn thành **gia sư có tool, context, memory và policy** — không chỉ là chatbot.

- **CLI:** `tez`
- **Desktop:** Tauri 2 + React — bàn học local-first, quản lý nhiều provider, skills, MCP, plugin
- **Core:** Rust workspace, SQLite, credential vault
- **Môn:** Toán, Vật lý, Hóa học, Tiếng Anh
- **Provider:** mọi endpoint OpenAI-compatible (Groq, OpenRouter, OpenAI, Gemini, Ollama, custom), lưu nhiều kết nối và chuyển active

## Cài đặt (Linux)

Cài cho user hiện tại, không cần `sudo`:

```bash
./scripts/install-local.sh
```

Script build bản release rồi cài:

| Thành phần | Vị trí |
|---|---|
| CLI `tez` | `~/.local/bin/tez` |
| Desktop | `~/.local/opt/tez/` — mở từ menu **tez — Gia sư AI** |
| Desktop entry | `~/.local/share/applications/tez.desktop` |
| Icon | `~/.local/share/icons/hicolor/512x512/apps/tez.png` |

Yêu cầu build: Rust stable, Node + pnpm, WebKitGTK/GTK3 dev packages.

Chạy trực tiếp AppImage vừa build (không cần cài):

```bash
./apps/desktop/src-tauri/target/release/bundle/appimage/tez_0.1.0_amd64.AppImage
```

Gỡ cài đặt:

```bash
rm -rf ~/.local/opt/tez ~/.local/bin/tez \
  ~/.local/share/applications/tez.desktop \
  ~/.local/share/icons/hicolor/512x512/apps/tez.png
```

Dữ liệu học tập nằm ở `~/.local/share/teaching-ez/`; xóa thêm thư mục đó nếu muốn xóa toàn bộ session, tài liệu và key.

## Trạng thái hiện tại

Đã chạy được vertical slice:

| Thành phần | Trạng thái |
|---|---|
| Rust workspace (`tez-llm`, `tez-core`, `tez-store`, `tez-auth`, `tez-cli`) | ✅ |
| Agent loop: tool call, timeout 10s, cap output 8KB, tối đa 8 bước | ✅ |
| 4 subject pack + allowlist tool theo môn | ✅ |
| 9 tool: tính toán, kiểm tra bước, đổi đơn vị, đọc hiểu, khối lượng mol, cân bằng phương trình, vẽ đồ thị, công thức lý, ngữ pháp | ✅ |
| SQLite: session, message, tool call, token usage | ✅ |
| Nhiều provider đã lưu, key riêng từng provider, chuyển active | ✅ |
| Skills Markdown: 4 built-in, khớp môn + từ khóa, chèn vào context | ✅ |
| MCP stdio client: `initialize`, `tools/list`, `tools/call`, tool trace | ✅ |
| Plugin `plugin.json`: gói skills + MCP server, plugin mẫu | ✅ |
| Chat tương tác + resume session (`--session`) | ✅ |
| RAG văn bản: ingest `.txt`/`.md`, chunk + FTS5, tự chèn 4 đoạn liên quan vào chat | ✅ |
| Vault AES-256-GCM cho API key | ✅ |
| `tez connect`, `test`, `chat`, `doctor`, `subject`, `sessions`, `usage`, `doc`, `tools`, `provider`, `skills`, `mcp`, `plugins` | ✅ |
| Disk quota + cleanup (`scripts/disk-guard.sh`) | ✅ |
| RAG PDF/DOCX, context inspector | ⏳ |
| Tauri desktop: chat, chọn môn/lớp, tool trace, lịch sử phiên | ✅ |
| Desktop: quản lý provider, kiểm tra kết nối, skills/MCP/plugin, dữ liệu | ✅ |
| Đóng gói AppImage (`scripts/desktop.sh appimage`) | ✅ |

## Build & test

Yêu cầu: Rust stable, Node + pnpm. Desktop cần thêm WebKitGTK/GTK3 dev packages (xem `docs/DESKTOP-CLI.md`).

```bash
./scripts/build.sh        # disk guard + cargo build
./scripts/test.sh         # disk guard + fmt + clippy + test
```

Binary: `target/debug/tez`

## Dùng nhanh

```bash
# Kiểm tra môi trường, DB, vault, tool
./target/debug/tez doctor

# Kết nối provider (nhập API key ẩn)
./target/debug/tez connect --provider groq

# Hoặc Ollama local, không cần key
./target/debug/tez connect --provider ollama --model qwen3:4b

# Kiểm tra provider/model/key đã lưu
./target/debug/tez test

# Chat một lần
./target/debug/tez chat --subject math "Giải thích định lý Pythagore"

# Chat tương tác (giữ lịch sử trong session)
./target/debug/tez chat --subject math

# Chat tiếp một session cũ
./target/debug/tez chat --subject math --session <session-id> "Em chưa hiểu bước 2"

./target/debug/tez chat --subject chemistry "Cân bằng Fe + O2 -> Fe2O3"
./target/debug/tez chat --subject physics "Một xe chạy 54 km/h, đổi sang m/s"
./target/debug/tez chat --subject english "Sửa lỗi: i have a apple"

# Xem lại
./target/debug/tez sessions
./target/debug/tez usage
```

`--subject` nhận cả id (`math`, `physics`, `chemistry`, `english`) và tên tiếng Việt (`Toán`, `Lý`, `Hóa`, `Anh`).

### Nhiều provider

`tez connect` thêm một kết nối vào sổ provider thay vì ghi đè kết nối cũ. Mỗi provider có key mã hóa riêng; chỉ một provider đang active tại một thời điểm.

```bash
./target/debug/tez provider list
./target/debug/tez provider use groq-a1b2c3d4
./target/debug/tez provider remove ollama-e5f6a7b8

# Thêm provider thứ hai
./target/debug/tez connect --provider ollama --model qwen3:4b
```

Desktop dùng cùng sổ provider: **Cài đặt → Nhà cung cấp** để thêm, sửa, kiểm tra, chuyển hoặc xóa.

### Tài liệu (RAG)

`tez chat` tự tìm 4 đoạn liên quan nhất trong kho tài liệu của môn đang học và chèn vào context (có nhãn `<doc>` + tiêu đề để trích dẫn).

```bash
# Thêm ghi chú / tóm tắt / đề cương dạng txt hoặc md
./target/debug/tez doc add notes/pythagore.md --subject math --title "Định lý Pythagore"

./target/debug/tez doc list
./target/debug/tez doc search "tam giac vuong" --subject math
./target/debug/tez doc remove <document-id>
```

Hiện hỗ trợ UTF-8 `.txt`/`.md`. PDF/DOCX sẽ thêm ở bước sau (cần extract text + OCR cho ảnh scan).

### Tool

```bash
./target/debug/tez tools list
./target/debug/tez tools list --subject chemistry

./target/debug/tez tools test molar_mass \
  --args '{"formula":"CuSO4·5H2O"}' --subject chemistry

./target/debug/tez tools test balance_equation \
  --args '{"equation":"C3H8 + O2 -> CO2 + H2O"}' --subject chemistry

./target/debug/tez tools test plot_function \
  --args '{"expression":"x^2 - 2*x + 1","x_min":-2,"x_max":4}' --subject math
```

### Skills, MCP và plugin

```bash
# Skill Markdown trong ~/.local/share/teaching-ez/skills/
./target/debug/tez skills list
./target/debug/tez skills show step-by-step

# MCP server stdio trong mcp.json hoặc từ plugin
./target/debug/tez mcp list
./target/debug/tez mcp test study

# Plugin local trong ~/.local/share/teaching-ez/plugins/
./target/debug/tez plugins
```

- **Skill** là prompt pack, được khớp theo môn + từ khóa và chèn vào system prompt (tối đa 2 skill/lượt).
- **MCP** là tool ngoài qua stdio JSON-RPC; tên tool có dạng `mcp__<server>__<tool>`.
- **Plugin** là gói `plugin.json` chứa skills + MCP server, không chạy code tùy ý.

Plugin mẫu nằm ở `examples/plugins/study-toolkit/` (skill phòng thi + MCP `echo`/`add`). Trong desktop bấm **Cài đặt → Plugin → Cài plugin mẫu**.

Chi tiết định dạng file và bảo mật: [docs/EXTENSIONS.md](docs/EXTENSIONS.md).

### Subject pack

```bash
./target/debug/tez subject list
./target/debug/tez subject show chemistry
```

Mỗi pack khai báo prompt sư phạm, chiến lược dạy và **allowlist tool**. Agent chỉ thấy tool của môn đang học.

## Cấu trúc

```
crates/
  tez-llm/      provider trait + OpenAI-compatible adapter
  tez-core/     agent loop, context builder, subject packs, tools, skills, MCP, plugin
  tez-store/    SQLite schema + session/usage/document repositories
  tez-auth/     AES-256-GCM vault + sổ nhiều provider
  tez-cli/      binary `tez`
apps/
  desktop/      React + Vite + Tauri 2
examples/
  plugins/      plugin mẫu (skills + MCP server)
docs/           kế hoạch, kiến trúc, provider auth, extensions, disk policy
scripts/        disk guard, build/test/desktop/install wrappers
```

Dữ liệu người dùng nằm ở `~/.local/share/teaching-ez/`:

- `tez.db` — session, message, tool call, usage, tài liệu + FTS5 index
- `providers.json` — sổ nhiều provider + provider đang dùng
- `connection.json` — bản đồng bộ của provider active (tương thích CLI cũ)
- `mcp.json` — cấu hình MCP server
- `skills/` — skill Markdown của user
- `plugins/` — plugin local
- `master.key` — khóa vault (quyền 0600)

## Dung lượng

Mọi thứ tự lớn (toolchain, cache, build, log, DB, model Ollama) đều có hạn mức mềm/cứng và quy trình dọn dẹp:

```bash
./scripts/disk-guard.sh check
./scripts/disk-guard.sh clean
./scripts/disk-guard.sh auto
```

Chi tiết: [docs/DISK-POLICY.md](docs/DISK-POLICY.md)

## Desktop (Tauri)

Desktop dùng chung dữ liệu với CLI tại `~/.local/share/teaching-ez/`. Có thể kết nối provider ngay trong app bằng nút **Cài đặt** (hoặc dùng CLI nếu thích):

```bash
./target/debug/tez connect --provider groq

cd apps/desktop
pnpm install          # lần đầu
cd ../..
scripts/desktop.sh dev             # chạy app
scripts/desktop.sh check           # build frontend + cargo check/clippy
scripts/desktop.sh appimage        # đóng gói AppImage (release)
scripts/desktop.sh appimage-debug  # AppImage bản debug, nhanh hơn
```

**Sử dụng:** mở **tez — Gia sư AI** từ menu → **Cài đặt → Nhà cung cấp** → chọn preset (Groq, Gemini, OpenRouter, OpenAI, Ollama) hoặc Custom → dán API key (để trống nếu endpoint không cần key) → **Lưu nhà cung cấp** → **Kiểm tra**. Sau đó chọn môn/lớp và nhập câu hỏi.

Giao diện theo hướng bàn học editorial: nền giấy ấm, chữ serif cho tiêu đề, màu xanh rừng + đất nung, icon SVG tự vẽ, không gradient/pill/emoji. Các khu vực:

- **Chat:** chọn môn/lớp, phiên mới, tải lại lịch sử, tool trace có args/kết quả/thời gian, token usage.
- **Cài đặt → Nhà cung cấp:** nhiều provider, key riêng, chuyển active, sửa, xóa, kiểm tra latency.
- **Cài đặt → Kỹ năng:** xem skill, môn, từ khóa, nội dung Markdown, mở thư mục.
- **Cài đặt → MCP:** thêm/sửa/xóa server, env, cwd, test và xem danh sách tool.
- **Cài đặt → Plugin:** cài plugin mẫu, xem skills/MCP đi kèm, mở thư mục.
- **Cài đặt → Dữ liệu:** đường dẫn, số session, mở nhanh các thư mục cấu hình.

AppImage release được build bởi `scripts/install-local.sh` tại:

```
apps/desktop/src-tauri/target/release/bundle/appimage/tez_0.1.0_amd64.AppImage
```

Chạy không cần FUSE:

```bash
./apps/desktop/src-tauri/target/release/bundle/appimage/tez_0.1.0_amd64.AppImage --appimage-extract-and-run
```

## Giới hạn hiện tại

- Chưa streaming; câu trả lời hiện về một lần.
- RAG chỉ nhận `.txt`/`.md`; PDF/DOCX/OCR chưa có.
- Vault dùng `master.key` trong thư mục dữ liệu, chưa chuyển sang OS keychain.
- MCP chỉ hỗ trợ stdio; server spawn mỗi lượt chat, timeout 20 giây, chưa sandbox.
- Plugin là gói cấu hình; không có code plugin tùy ý ngoài MCP server do bạn khai báo.
- Skill khớp từ khóa đơn giản, tối đa 2 skill mỗi lượt.
- AppImage release cho Linux đã đóng gói; installer cho Windows/macOS sẽ làm sau.
- Chưa có sync đa máy; dữ liệu nằm trên máy bạn.

## Tài liệu

- [Kế hoạch xây dựng](docs/PLAN.md)
- [Kiến trúc Rust + Agent](docs/RUST-ARCHITECTURE.md)
- [Desktop app + CLI](docs/DESKTOP-CLI.md)
- [Kết nối AI (BYOK)](docs/PROVIDER-AUTH.md)
- [Skills, MCP và plugin](docs/EXTENSIONS.md)
- [Chính sách dung lượng](docs/DISK-POLICY.md)

## Roadmap gần

1. RAG PDF/DOCX + OCR, `context inspect` để xem token từng phần.
2. Lưu connection vào SQLite + OS keychain (thay master key file).
3. Thêm tool Anh (`vocabulary_lookup`) và eval 8 case/môn.
4. Streaming SSE cho desktop.
5. Đóng gói release cho Windows/macOS + auto-update.
