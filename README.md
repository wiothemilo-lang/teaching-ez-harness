# teaching-ez-harness

[![CodeRabbit Reviews](https://img.shields.io/badge/CodeRabbit-reviewed-FF570A?logo=coderabbit&logoColor=white)](https://coderabbit.ai)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app/)
[![Tests](https://img.shields.io/badge/tests-362%20passing-brightgreen)](#build--test)
[![Version](https://img.shields.io/badge/version-0.1.0-blue)](Cargo.toml)
[![Local-first](https://img.shields.io/badge/local--first-BYOK-2F7D5B)](docs/DESKTOP-CLI.md)
[![Platform](https://img.shields.io/badge/platform-Linux%20%C2%B7%20Windows-lightgrey?logo=linux&logoColor=black)](#desktop-tauri)
[![License](https://img.shields.io/badge/license-MIT-yellow)](LICENSE)

Harness local-first để biến model AI của bạn thành **workspace có tool, context, memory và policy** — không chỉ là chatbot.

- **CLI:** `tez`
- **Desktop:** Tauri 2 + React — giao diện tối kiểu Claude Code, ba chế độ **Chat**, **Plan** và **Code**
- **Chat:** trợ lý tổng quát, **không giới hạn môn học**; Toán/Lý/Hóa/Anh chỉ là preset tùy chọn
- **Code:** agent đọc/ghi file, liệt kê thư mục và chạy lệnh trong workspace được chọn
- **Core:** Rust workspace, SQLite, credential vault
- **Provider:** mọi endpoint OpenAI-compatible (Groq, OpenRouter, OpenAI, Gemini, Ollama, custom), lưu nhiều kết nối và chuyển active
- **Cập nhật:** kiểm tra GitHub Releases trong app hoặc `tez update`

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

Yêu cầu build: Rust stable, Node + pnpm, WebKitGTK/GTK3 dev packages. Cần thêm gói `bubblewrap` cho sandbox `run_command`/MCP stdio trên Linux (ví dụ `sudo apt install bubblewrap`).

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

## Cài đặt (Windows)

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1
```

Script tải bản mới nhất từ GitHub Releases, cài vào `%LOCALAPPDATA%\tez`, tạo shortcut Start Menu và thêm vào PATH. Sau đó mở **tez** từ Start Menu hoặc chạy `scripts\windows\tez.bat`.

Build từ source trên Windows:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\build.ps1
```

Chi tiết: [docs/WINDOWS.md](docs/WINDOWS.md).

## Trạng thái hiện tại

Đã chạy được vertical slice:

| Thành phần | Trạng thái |
|---|---|
| Rust workspace (`tez-llm`, `tez-core`, `tez-store`, `tez-auth`, `tez-update`, `tez-cli`) | ✅ |
| Agent loop: tool call, timeout 60s, cap output 8KB, tối đa 12 bước (chỉnh được) | ✅ |
| Chat tổng quát (không giới hạn môn) + 4 preset Toán/Lý/Hóa/Anh; gửi kèm ảnh | ✅ |
| Plan mode: chỉ đọc workspace, khảo sát và trình kế hoạch theo 6 mục bắt buộc; lưu plan vào `plans/<session>.md`; desktop có nút Kế hoạch mở editor, cảnh báo mục thiếu và chặn lưu nếu chưa đủ (có Lưu nháp) | ✅ |
| Chế độ Code: `read_file`, `write_file`, `edit_file`, `list_files`, `glob`, `grep`, `run_command` trong workspace | ✅ |
| Harness: permission pipeline 4 mức + rule theo tool/path, approval kèm diff, ghi nhớ approval theo phiên (TTL/số mục tùy chỉnh), hook audit + che bí mật | ✅ |
| Harness: todo plan, subagent `task`, compaction 4 bước, memory dài hạn | ✅ |
| `harness.json` + tab **Cài đặt → Agent** (quyền, model, system prompt, ngôn ngữ, nén, memory, hook, sandbox lệnh) | ✅ |
| 9 tool học tập: tính toán, kiểm tra bước, đổi đơn vị, đọc hiểu, khối lượng mol, cân bằng phương trình, vẽ đồ thị, công thức lý, ngữ pháp | ✅ |
| SQLite: session (kèm mode), message, tool call, todo, token usage | ✅ |
| Nhiều provider đã lưu, key riêng từng provider, chuyển active | ✅ |
| Skills Markdown: 4 skill seed, catalog rút gọn theo nguồn + tool `skill` (không tên = danh mục đầy đủ, có tên = nội dung), tạo bằng AI | ✅ |
| Ngữ cảnh dự án: `AGENTS.md`/`CLAUDE.md`, `.tez/skills`, `.tez/mcp.json` | ✅ |
| Web: tool `web_fetch` tải http/https, chặn host nội bộ/DNS rebinding, pin toàn bộ địa chỉ DNS, redirect kiểm tra lại (static), lọc HTML, cap 512 KB (static); `render: "browser"` tự fallback static khi thiếu/lỗi, resolver map toàn bộ địa chỉ, `--no-sandbox` chỉ khi `TEZ_BROWSER_NO_SANDBOX=1` | ✅ |
| Ảnh: đính kèm PNG/JPEG/WebP/GIF (tối đa 4 ảnh, 5 MB/ảnh, tổng 8 MB/lượt), ghi file trong `images/<session>/`; SQLite giữ ref `tez-image:`; gửi lại tối đa 2 lượt user gần nhất | ✅ |
| MCP client: stdio hoặc Streamable HTTP (`url` + headers; header hỗ trợ `${VAR}` và `bearer_token_env`), `initialize`, `tools/list`, `tools/call`, resources/prompts qua tool `mcp__<server>__extras`, tool trace; server/tool read-only dùng được trong Plan | ✅ |
| Plugin `plugin.json`: gói skills + MCP server, plugin mẫu, tạo bằng AI | ✅ |
| Eval hồi quy provider: bộ JSON `evals/*.json`, `tez eval`, CI nightly tùy chọn | ✅ |
| Chat tương tác + resume session (`--session`) | ✅ |
| RAG văn bản: ingest `.txt`/`.md`, chunk + FTS5, tự chèn 4 đoạn liên quan vào chat | ✅ |
| Vault AES-256-GCM cho API key | ✅ |
| `tez connect`, `test`, `eval`, `chat`, `update`, `doctor`, `backup`, `restore`, `logs`, `subject`, `sessions`, `usage`, `doc`, `tools`, `provider`, `skills`, `mcp`, `plugins`, `memory`, `harness` | ✅ |
| Disk quota + cleanup (`scripts/disk-guard.sh`) | ✅ |
| RAG PDF/DOCX, context inspector | ⏳ |
| Tauri desktop: ba chế độ Chat/Plan/Code, tool trace, lịch sử phiên, todo/context dock | ✅ |
| Desktop: giao diện tối kiểu Claude Code, command palette `Ctrl K`, tạo skill/MCP/plugin bằng AI | ✅ |
| Kiểm tra + tải cập nhật từ GitHub Releases (desktop & CLI) | ✅ |
| Đóng gói AppImage Linux + installer Windows (NSIS/MSI) | ✅ |

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

# Chạy bộ eval hồi quy (mặc định evals/basic.json)
./target/debug/tez eval
./target/debug/tez eval --json --min-pass 8

# Chat tổng quát — không cần chọn môn
./target/debug/tez chat "Giải thích vì sao bầu trời màu xanh"

# Chat tương tác (giữ lịch sử trong session)
./target/debug/tez chat

# Chat tiếp một session cũ
./target/debug/tez chat --session <session-id> "Em chưa hiểu bước 2"

# Preset môn học vẫn dùng được khi muốn
./target/debug/tez chat --subject math "Giải thích định lý Pythagore"
./target/debug/tez chat --subject chemistry "Cân bằng Fe + O2 -> Fe2O3"

# Chế độ Code trong một workspace
./target/debug/tez chat --mode code --workspace ~/du-an "Đọc cấu trúc và tìm lỗi build"
./target/debug/tez chat --mode code --workspace ~/du-an

# Plan trước, lưu kế hoạch vào plans/<session>.md, rồi chuyển Code
./target/debug/tez chat --mode plan --workspace ~/du-an "Lập kế hoạch sửa lỗi build"

# Harness: xem cấu hình, duyệt tự động, giới hạn bước, chế độ chỉ đọc
./target/debug/tez harness
./target/debug/tez chat --yes "Chạy test và sửa lỗi"
./target/debug/tez chat --permission read_only "Giải thích kiến trúc repo"
./target/debug/tez chat --max-steps 20 --no-memory "Tiếp tục việc còn dở"

# Bộ nhớ dài hạn
./target/debug/tez memory list
./target/debug/tez memory search "quy ước build"

# Kiểm tra / tải cập nhật
./target/debug/tez update
./target/debug/tez update --download

# Sao lưu / phục hồi dữ liệu local
./target/debug/tez backup
./target/debug/tez backup --list
./target/debug/tez restore ~/.local/share/teaching-ez/backups/tez-backup-... --yes

# Xem log gần nhất (desktop ghi cùng file)
./target/debug/tez logs --tail 100

# Xem lại
./target/debug/tez sessions
./target/debug/tez usage
```

`--subject` nhận id (`general`, `math`, `physics`, `chemistry`, `english`) hoặc tên tiếng Việt (`Toán`, `Lý`, `Hóa`, `Anh`). Bỏ trống = trợ lý tổng quát. Giá trị không khớp cũng rơi về tổng quát thay vì báo lỗi.

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

# MCP server (stdio hoặc HTTP) trong mcp.json hoặc từ plugin
./target/debug/tez mcp list
./target/debug/tez mcp test study

# Plugin local trong ~/.local/share/teaching-ez/plugins/
./target/debug/tez plugins
```

- **Skill** là prompt pack Markdown. System prompt chỉ nhận catalog rút gọn theo nguồn (tối đa 12 id/nguồn); model gọi `skill` không kèm `name` để xem danh mục đầy đủ, hoặc kèm `name` để nạp toàn bộ nội dung (tối đa 12.000 ký tự). Môn học chỉ còn là bộ lọc tùy chọn cho RAG và preset CLI.
- **MCP** là tool ngoài qua stdio JSON-RPC hoặc Streamable HTTP (`url`, `headers` hỗ trợ `${VAR}`, `bearer_token_env` đọc lúc connect, session `Mcp-Session-Id`, nhận JSON/SSE); tên tool có dạng `mcp__<server>__<tool>`. Server có thể đặt `read_only` (alias `readOnly`) hoặc tool khai báo `annotations.readOnlyHint`; chỉ các tool này được đăng ký trong Plan. Server có capability `resources`/`prompts` được cấp thêm tool read-only `mcp__<server>__extras` với các action `list_resources`, `read_resource`, `list_prompts`, `get_prompt`.
- **Plugin** là gói `plugin.json` chứa skills + MCP server, không chạy code tùy ý.

Plugin mẫu nằm ở `examples/plugins/study-toolkit/` (skill phòng thi + MCP `echo`/`add`). Trong desktop bấm **Cài đặt → Plugin → Cài plugin mẫu**.

Chi tiết định dạng file và bảo mật: [docs/EXTENSIONS.md](docs/EXTENSIONS.md).

### Subject pack (tùy chọn)

```bash
./target/debug/tez subject list
./target/debug/tez subject show chemistry
```

Pack `general` là mặc định và không giới hạn chủ đề. Bốn pack Toán/Lý/Hóa/Anh là preset: mỗi pack khai báo prompt sư phạm, chiến lược dạy và allowlist tool. Ở chế độ Chat, agent dùng tool của pack đang chọn; ở chế độ Code, agent dùng bộ tool workspace (`read_file`, `write_file`, `edit_file`, `list_files`, `glob`, `grep`, `run_command`) cùng `todo_write`, `task`, `compact` và memory.

## Cấu trúc

```
crates/
  tez-llm/      provider trait + OpenAI-compatible adapter
  tez-core/     agent loop, context builder, subject packs, tools, skills, MCP, plugin
  tez-store/    SQLite schema + session/usage/document repositories
  tez-auth/     AES-256-GCM vault + sổ nhiều provider
  tez-backup/   snapshot DB + manifest SHA-256, phục hồi có kiểm tra
  tez-update/   kiểm tra/tải bản phát hành từ GitHub Releases
  tez-cli/      binary `tez`
apps/
  desktop/      React + Vite + Tauri 2
evals/          bộ eval hồi quy provider (JSON)
examples/
  plugins/      plugin mẫu (skills + MCP server)
docs/           kế hoạch, kiến trúc, provider auth, extensions, disk policy
scripts/        disk guard, build/test/desktop/install wrappers
```

Dữ liệu người dùng nằm ở `~/.local/share/teaching-ez/`:

- `tez.db` — session, message, tool call, todo, usage, tài liệu + FTS5 index
- `providers.json` — sổ nhiều provider + provider đang dùng
- `connection.json` — bản đồng bộ của provider active (tương thích CLI cũ)
- `mcp.json` — cấu hình MCP server
- `skills/` — skill Markdown của user
- `plugins/` — plugin local
- `harness.json` — permission mode, giới hạn agent, compaction, memory, hook, sandbox lệnh
- `memory/` — bộ nhớ dài hạn (mỗi bản ghi một file Markdown + `MEMORY.md`)
- `audit.jsonl` — audit log tool call và lần kết thúc phiên (xoay sang `audit.jsonl.1` khi vượt 5 MB)
- `logs/tez.log` — log ứng dụng/panic của CLI và desktop (xoay sang `tez.log.1` khi vượt 5 MB)
- `backups/` — bản sao lưu do `tez backup` tạo; chứa cả `master.key` nên phải bảo vệ như dữ liệu gốc
- `sessions/<id>/` — artifact của compaction (tool result lớn, transcript)
- `plans/<session>.md` — kế hoạch cuối của phiên Plan (tối đa 200.000 ký tự)
- `master.key` — khóa vault (quyền 0600)

## Dung lượng

Mọi thứ tự lớn (toolchain, cache, build, log, DB, model Ollama) đều có hạn mức mềm/cứng và quy trình dọn dẹp:

- **Rust build tổng ≤ 30GB** (mềm 24GB): vượt mềm dọn incremental/cache cũ, vượt cứng `cargo clean`.
- Tự chạy trước mỗi build và mỗi 6 giờ qua cron; không dọn khi `cargo`/`rustc` đang chạy.

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

**Sử dụng:** mở **tez — Chat & Code** từ menu → **Cài đặt → Model** → chọn preset (Groq, Gemini, OpenRouter, OpenAI, Ollama) hoặc Custom → dán API key (để trống nếu endpoint không cần key) → **Lưu nhà cung cấp** → **Kiểm tra**.

Giao diện tối lấy cảm hứng từ Claude Code: nền than ấm, viền mảnh, nhãn monospace, accent cam đất, không gradient rực rỡ. Sidebar gọn với workspace, danh sách phiên và công tắc **Chat / Plan / Code**; `Ctrl K` mở command palette để chuyển chế độ, mở cài đặt hoặc tạo extension.

- **Chat:** trợ lý tổng quát, không bắt chọn môn hay lớp; câu trả lời stream dần, nút gửi đổi thành nút **dừng** khi agent đang chạy; tool trace có args/kết quả/thời gian, token usage.
- **Plan:** chỉ đọc workspace; agent khảo sát, đặt câu hỏi và trình kế hoạch (mục tiêu, file, bước, rủi ro, cách kiểm chứng) trước khi bạn chuyển sang Code. Kế hoạch cuối được lưu vào `plans/<session>.md` (tối đa 200.000 ký tự); khi chuyển Plan → Code, desktop nạp lại plan, giữ nguyên phiên, todos và hội thoại. Nút Kế hoạch trên header mở PlanDialog để xem, sửa và lưu plan; **Lưu kế hoạch** yêu cầu đủ 6 mục, **Lưu nháp** giữ bản chưa hoàn chỉnh.
- **Code:** nhập hoặc mở workspace; agent dùng `read_file`, `write_file`, `edit_file`, `list_files`, `glob`, `grep`, `run_command`, `todo_write`, `task`, `compact` và bị giới hạn trong thư mục đó.
- **Ngữ cảnh dự án:** khi đã chọn workspace, agent đọc `AGENTS.md`/`CLAUDE.md` (và bản trong `.tez/`), nạp skill ở `.tez/skills/` và MCP server ở `.tez/mcp.json`; server dự án thắng server toàn cục khi trùng tên.
- **Duyệt thao tác:** khi permission mode hoặc rule yêu cầu, app hiện hộp thoại kèm lý do, tool và **diff trước khi ghi** cho `write_file`/`edit_file`; chọn **Từ chối** hoặc **Cho phép chạy**. Tick **Cho phép thao tác này trong phiên hiện tại** để các lần sau cùng tool + path/command trong phiên được tự duyệt.
- **Dock harness:** ngay trên ô nhập, hiện kế hoạch todo, thanh context so với giới hạn, số lượt nén và số thao tác bị chặn.
- **Cài đặt → Agent:** chọn permission mode (chỉ đọc / nghiêm ngặt / tiêu chuẩn / tin cậy), viết **permission rule** dạng `allow|ask|deny <tool> <pattern>` (dòng trống và `#` comment được bỏ qua; lỗi hiện ngay và nút **Lưu** bị tắt khi rule chưa hợp lệ), giới hạn bước/token/temperature/top_p, output tool tối đa, số bước subagent, TTL và số mục approval đã nhớ (kèm nút xoá), system prompt bổ sung, ngôn ngữ trả lời, danh sách tool bị tắt, compaction, memory, hook shell và xoá bản ghi memory.
- **Cài đặt → Model:** nhiều provider, key riêng, chuyển active, sửa, xóa, kiểm tra latency.
- **Cài đặt → Kỹ năng:** xem skill, ngữ cảnh, từ khóa, nội dung Markdown, mở thư mục, hoặc **Tạo với AI** (mô tả → xem trước `SKILL.md` → lưu).
- **Cài đặt → MCP:** thêm/sửa/xóa server stdio hoặc Streamable HTTP (`URL`, `headers`, `Bearer token env`), env, cwd, test và xem danh sách tool; đánh dấu **Chỉ đọc (cho phép trong Plan)**; bật **Sandbox stdio** (Linux, bubblewrap) với mạng chặn mặc định và danh sách thư mục ghi được; **Tạo với AI** sinh cấu hình stdio.
- **Cài đặt → Plugin:** cài plugin mẫu, xem skills/MCP đi kèm, mở thư mục; **Tạo với AI** sinh `plugin.json` + skill.
- **Cài đặt → Cập nhật:** kiểm tra GitHub Releases, xem ghi chú, tải installer phù hợp, mở file đã tải.
- **Cài đặt → Dữ liệu:** đường dẫn, số session, mở nhanh các thư mục cấu hình, tạo bản sao lưu và phục hồi từ thư mục backup có `manifest.json`.

AppImage release được build bởi `scripts/install-local.sh` tại:

```
apps/desktop/src-tauri/target/release/bundle/appimage/tez_0.1.0_amd64.AppImage
```

Chạy không cần FUSE:

```bash
./apps/desktop/src-tauri/target/release/bundle/appimage/tez_0.1.0_amd64.AppImage --appimage-extract-and-run
```

## Giới hạn hiện tại

- Streaming phát text theo từng đoạn; tool call vẫn được gom đủ trong một lượt. Nút dừng có hiệu lực giữa các bước; khi huỷ, `run_command` dừng cả process group nên process con không tiếp tục chạy.
- Gọi model tự thử lại tối đa 2 lần với backoff cho HTTP 429/5xx và lỗi kết nối/timeout; tôn trọng `Retry-After` (tối đa 10 giây). Stream chỉ thử lại trước khi server gửi byte đầu tiên.
- RAG chỉ nhận `.txt`/`.md`; PDF/DOCX/OCR chưa có.
- `web_fetch` mặc định tải HTML tĩnh, không đảm bảo render đủ JavaScript. `render: "browser"` dùng Chromium/Chrome/Edge headless nếu có (`TEZ_CHROMIUM`, hoặc `chromium`/`chrome`/`msedge` trên `PATH`), timeout 25 s; nếu thiếu browser hoặc browser lỗi, tool tự fallback sang static và ghi rõ lý do. `--no-sandbox` chỉ bật khi `TEZ_BROWSER_NO_SANDBOX=1`. HTML parser vẫn đơn giản (comment/entity số/script tự đóng đã xử lý nhưng thẻ lỗi có thể còn sót); DNS được resolve và pin toàn bộ địa chỉ ở static; browser map toàn bộ địa chỉ qua `--host-resolver-rules` nhưng không revalidate từng redirect; charset UTF-8/Latin-1/Windows-1252; body static cap 512 KB; PDF/audio/video bị từ chối ở static và chưa được trích xuất.
- Ảnh được ghi thành file trong `images/<session>/<message>-<index>.<ext>`; SQLite chỉ giữ ref `tez-image:` và `load_messages` giải mã lại thành data URL khi resume. DB nhỏ hơn nhiều so với lưu data URL, nhưng thư mục ảnh vẫn lớn dần. UI và backend giới hạn 4 ảnh mới/lượt; UI giới hạn 5 MB/ảnh, backend giới hạn tổng 8 MB/request. Khi gửi lại lịch sử, chỉ ảnh của tối đa 2 lượt user gần nhất được gửi, trong ngân sách 6 ảnh của cả request (ảnh hiện tại được ưu tiên). Ảnh cũ vẫn hiện trong UI/resume nhưng không được gửi lại toàn bộ; chưa hỗ trợ PDF/audio/video.
- Vault dùng `master.key` trong thư mục dữ liệu, chưa chuyển sang OS keychain.
- MCP hỗ trợ stdio và Streamable HTTP; server stdio spawn mỗi lượt chat, timeout 20 giây, có thể bật sandbox bubblewrap theo từng server (`sandbox.enabled`): mặc định chặn mạng, ẩn secret trong data dir, chỉ bind thêm các đường dẫn trong `writable_paths`; process chỉ kế thừa env cơ bản, không kế thừa API key/token của app. HTTP dùng POST JSON-RPC + response JSON/SSE; hỗ trợ OAuth 2.1 + PKCE với redirect loopback, token lưu ngoài `mcp.json`; chưa hỗ trợ GET stream server-initiated. Header hỗ trợ `${VAR}` và `bearer_token_env`. `mcp.json` hỏng được bỏ qua kèm cảnh báo thay vì chặn phiên chat; file hỏng được sao lưu trước khi ghi đè. Plan chỉ đăng ký tool MCP được khai báo read-only; app tin vào `readOnly`/`readOnlyHint` của server, không xác minh hành vi thực tế, và tool read-only vẫn có thể đọc dữ liệu ngoài workspace tùy server.
- Plan lưu nguyên văn câu trả lời cuối vào `plans/<session>.md` (tối đa 200.000 ký tự); desktop chỉ nạp lại khi chuyển trực tiếp Plan → Code, CLI nạp ở mỗi lượt Code (tối đa 20.000 ký tự khi chèn). Kế hoạch có schema Markdown 6 mục (`Mục tiêu`, `Phạm vi`, `File và thay đổi`, `Các bước`, `Rủi ro`, `Kiểm chứng`); desktop PlanDialog có form nhập từng mục bên cạnh Markdown đầy đủ, cảnh báo mục thiếu; **Lưu kế hoạch** chặn nếu chưa đủ, **Lưu nháp** cho phép giữ bản chưa hoàn chỉnh, CLI in cảnh báo ra stderr. Form giữ nguyên các mục Markdown khác ngoài 6 mục bắt buộc.
- **Cho phép thao tác này trong phiên hiện tại** được lưu vào `approvals.json` trong data dir; TTL và số mục tối đa chỉnh trong **Cài đặt → Agent** (mặc định 7 ngày / 200 mục), có nút xoá toàn bộ approval đã nhớ. Khóa theo `session_id|tool|path/command|fingerprint(args)`, nên cùng đường dẫn nhưng nội dung hoặc tham số khác sẽ hỏi lại. File corrupt bị bỏ qua và ghi log.
- Plugin là gói cấu hình; không có code plugin tùy ý ngoài MCP server do bạn khai báo.
- Skill dùng hai tầng: catalog rút gọn theo nguồn trong system prompt và tool `skill` (không tên = danh mục đầy đủ, có tên = body tối đa 12.000 ký tự). Skill vẫn là prompt pack, không chạy code; bốn skill seed nằm trong `skills/` nên hiện nguồn `user`.
- Code mode giới hạn đường dẫn ở tầng ứng dụng. Trên Linux, `run_command` mặc định chạy trong bubblewrap: workspace được bind read-write, data dir và credential trong HOME bị ẩn, network namespace bị tách (chặn egress) trừ khi bật `sandbox.allow_network`. Cần gói `bubblewrap`; nếu thiếu `bwrap`, lệnh bị từ chối thay vì chạy không sandbox. Trên macOS/Windows sandbox OS chưa có, nên chỉ mở workspace tin cậy.
- Permission pipeline và hook là lớp kiểm soát của app, không thay thế sandbox hệ điều hành; deny-list chỉ chặn các mẫu phá hủy rõ ràng.
- Memory dùng chấm điểm từ khoá, chưa có embedding; compaction có thể mất chi tiết nếu phải tóm tắt bằng model (transcript vẫn được lưu).
- Cập nhật không tự động cài: người dùng bấm tải rồi chạy installer.
- Installer Windows có trong workflow release nhưng chưa được ký số (SmartScreen có thể cảnh báo).
- Chưa có sync đa máy; dữ liệu nằm trên máy bạn.

## Tài liệu

- [Kế hoạch xây dựng](docs/PLAN.md)
- [Kiến trúc Rust + Agent](docs/RUST-ARCHITECTURE.md)
- [Desktop app + CLI](docs/DESKTOP-CLI.md)
- [Kết nối AI (BYOK)](docs/PROVIDER-AUTH.md)
- [Skills, MCP và plugin](docs/EXTENSIONS.md)
- [Chính sách dung lượng](docs/DISK-POLICY.md)
- [Đánh giá mức sẵn sàng production](docs/PRODUCTION-READINESS.md)
- [Windows: cài đặt, build, cập nhật](docs/WINDOWS.md)
- [Kiểm tra và cập nhật phiên bản](docs/UPDATING.md)

## Roadmap gần

1. RAG PDF/DOCX + OCR, `context inspect` để xem token từng phần.
2. Lưu connection vào SQLite + OS keychain (thay master key file).
3. Thêm tool Anh (`vocabulary_lookup`) và eval 8 case/môn.
4. MCP HTTP GET stream, multimodal đầy đủ (PDF/audio) và hoàn thiện render JavaScript cho `web_fetch` (sandbox, độ ổn định).
5. Ký số installer Windows và bật auto-update có xác minh chữ ký.
