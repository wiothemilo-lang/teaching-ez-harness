# teaching-ez-harness

[![CodeRabbit Reviews](https://img.shields.io/badge/CodeRabbit-reviewed-FF570A?logo=coderabbit&logoColor=white)](https://coderabbit.ai)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app/)
[![Tests](https://img.shields.io/badge/tests-42%20passing-brightgreen)](#build--test)
[![Version](https://img.shields.io/badge/version-0.1.0-blue)](Cargo.toml)
[![Local-first](https://img.shields.io/badge/local--first-BYOK-2F7D5B)](docs/DESKTOP-CLI.md)
[![Platform](https://img.shields.io/badge/platform-Linux%20%C2%B7%20AppImage-lightgrey?logo=linux&logoColor=black)](#desktop-tauri)
[![License](https://img.shields.io/badge/license-MIT-yellow)](LICENSE)

Harness local-first để biến model AI của bạn thành **gia sư có tool, context, memory và policy** — không chỉ là chatbot.

- **CLI:** `tez`
- **Desktop:** Tauri 2 + React (chat, cài đặt provider, lịch sử phiên)
- **Core:** Rust workspace, SQLite, credential vault
- **Môn:** Toán, Vật lý, Hóa học, Tiếng Anh
- **Provider:** mọi endpoint OpenAI-compatible (Groq, OpenRouter, OpenAI, Gemini, Ollama, custom)

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
| Chat tương tác + resume session (`--session`) | ✅ |
| RAG văn bản: ingest `.txt`/`.md`, chunk + FTS5, tự chèn 4 đoạn liên quan vào chat | ✅ |
| Vault AES-256-GCM cho API key | ✅ |
| `tez connect`, `test`, `chat`, `doctor`, `subject`, `sessions`, `usage`, `doc`, `tools` | ✅ |
| Disk quota + cleanup (`scripts/disk-guard.sh`) | ✅ |
| RAG PDF/DOCX, context inspector | ⏳ |
| Tauri desktop: chat, chọn môn/lớp, tool trace | ✅ |
| Desktop: cài đặt provider trong app, kiểm tra kết nối, lịch sử phiên | ✅ |
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
  tez-core/     agent loop, context builder, subject packs, tools
  tez-store/    SQLite schema + session/usage/document repositories
  tez-auth/     AES-256-GCM vault
  tez-cli/      binary `tez`
apps/
  desktop/      React + Vite + Tauri 2
docs/           kế hoạch, kiến trúc, provider auth, disk policy
scripts/        disk guard, build/test/desktop/install wrappers
```

Dữ liệu người dùng nằm ở `~/.local/share/teaching-ez/`:

- `tez.db` — session, message, tool call, usage, tài liệu + FTS5 index
- `connection.json` — provider + key đã mã hóa
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

**Sử dụng:** mở **tez — Gia sư AI** từ menu → bấm **Cài đặt** → chọn provider → dán API key (để trống nếu dùng Ollama) → **Lưu & kiểm tra** → chọn môn/lớp và nhập câu hỏi.

Giao diện hiện có: chọn môn/lớp, chat nhiều lượt, xem tool call (args/kết quả/thời gian), session mới, **lịch sử phiên** (bấm để tải lại), **cài đặt provider** (Groq, Gemini, OpenRouter, OpenAI, Ollama, custom) kèm nút lưu + kiểm tra kết nối.

AppImage debug đã build thử tại:

```
apps/desktop/src-tauri/target/debug/bundle/appimage/tez_0.1.0_amd64.AppImage
```

Chạy không cần FUSE:

```bash
./apps/desktop/src-tauri/target/debug/bundle/appimage/tez_0.1.0_amd64.AppImage --appimage-extract-and-run
```

## Giới hạn hiện tại

- Chưa streaming; câu trả lời hiện về một lần.
- RAG chỉ nhận `.txt`/`.md`; PDF/DOCX/OCR chưa có.
- Vault dùng `master.key` trong thư mục dữ liệu, chưa chuyển sang OS keychain.
- AppImage mới build ở bản debug để kiểm tra; installer release cho từng OS sẽ làm sau.
- Chưa có sync đa máy; dữ liệu nằm trên máy bạn.

## Tài liệu

- [Kế hoạch xây dựng](docs/PLAN.md)
- [Kiến trúc Rust + Agent](docs/RUST-ARCHITECTURE.md)
- [Desktop app + CLI](docs/DESKTOP-CLI.md)
- [Kết nối AI (BYOK)](docs/PROVIDER-AUTH.md)
- [Chính sách dung lượng](docs/DISK-POLICY.md)

## Roadmap gần

1. RAG PDF/DOCX + OCR, `context inspect` để xem token từng phần.
2. Lưu connection vào SQLite + OS keychain (thay master key file).
3. Thêm tool Anh (`vocabulary_lookup`) và eval 8 case/môn.
4. Streaming SSE cho desktop.
5. Đóng gói release cho Windows/macOS + auto-update.
