# Skills, MCP và plugin

> **Trạng thái:** MVP đã chạy trong CLI và desktop.
> **Nguyên tắc:** mọi thứ là file local, không cần build lại, không có marketplace hay code chạy ngầm ngoài MCP server do bạn khai báo.

---

## 1. Skills — prompt pack Markdown

Skill là một file `SKILL.md` chứa hướng dẫn cho agent. Tez dùng **hai tầng progressive disclosure**: system prompt chỉ nhận catalog rút gọn theo nguồn (mỗi nguồn một dòng, tối đa 12 id); khi phù hợp, model gọi tool `skill` không kèm `name` để xem danh mục đầy đủ theo nguồn, hoặc kèm `name` để nạp toàn bộ nội dung. Cách này giữ context nhẹ và cho model tự chọn skill. Skill không chạy code và không có quyền truy cập dữ liệu.

### Vị trí

```
~/.local/share/teaching-ez/skills/
  step-by-step/SKILL.md
  error-diagnosis/SKILL.md
  exam-review/SKILL.md
  english-writing/SKILL.md
```

Lần đầu mở app, bốn skill mặc định được tạo nếu thư mục chưa tồn tại. Vì chúng được seed ngay trong `skills/`, registry nạp chúng với nguồn `user` (không phải `builtin`).

### Định dạng

```markdown
---
name: Giải từng bước
description: Chia bài thành các bước nhỏ và kiểm tra từng bước.
subjects: math, physics, chemistry
triggers: giải, bước, chứng minh, tính
---
1. Tóm tắt dữ kiện trong 1-2 dòng.
2. Chia bài thành các bước nhỏ.
3. Sau mỗi bước, hỏi học sinh đã hiểu chưa.
```

| Trường | Ý nghĩa |
|---|---|
| `name` | Tên hiển thị. Nếu thiếu `id`, tên thư mục được dùng làm id. |
| `description` | Mô tả ngắn trong Cài đặt → Kỹ năng. |
| `subjects` | Danh sách id ngữ cảnh, phân tách bằng dấu phẩy. `all` hoặc bỏ trống = mọi ngữ cảnh. |
| `triggers` | Metadata từ khóa (không phân biệt hoa thường), dùng cho gợi ý/tương lai; catalog trong system prompt chỉ liệt kê tối đa 12 id/nguồn, danh mục đầy đủ lấy qua tool `skill`. |
| `id` | Tùy chọn, dùng để đặt id ổn định. |

Phần thân sau `---` được trả về trong thẻ `<skill id="…" name="…">…</skill>` khi model gọi `skill` với `name`, và bị cắt ở 12.000 ký tự. Danh mục đầy đủ (khi gọi `skill` không kèm `name`) bị cắt ở 24.000 ký tự.

### Hai tầng nạp

1. **System prompt:** `catalog_overview` gộp skill theo nguồn (`project`, `user`, `plugin`, `plugin:<id>`), mỗi nguồn một dòng kèm số lượng và tối đa 12 id tiêu biểu; phần dư hiện `… +N`.
2. **Tool `skill` không kèm `name`:** trả danh mục đầy đủ có tiêu đề `[nguồn] số_lượng`, tối đa 200 skill và 24.000 ký tự.
3. **Tool `skill` kèm `name`:** tìm theo id rồi tên, trả body tối đa 12.000 ký tự; tên sai trả danh sách id khả dụng.

### Skill mặc định

| Id | Ngữ cảnh | Dùng khi |
|---|---|---|
| `step-by-step` | Toán, Lý, Hóa | giải, bước, chứng minh, tính, bài tập |
| `error-diagnosis` | Toán, Lý, Hóa | sai, nhầm, lỗi, kiểm tra bước, không ra |
| `exam-review` | Mọi ngữ cảnh | ôn thi, ôn tập, đề, kiểm tra, thi |
| `english-writing` | Tiếng Anh | viết, sửa, essay, paragraph, grammar |

### CLI

```bash
tez skills list
tez skills show step-by-step
```

---

## 2. MCP — công cụ ngoài qua stdio hoặc HTTP

tez là MCP client tối giản: gọi `initialize`, `tools/list`, `tools/call`, rồi đóng kết nối khi lượt chat kết thúc. Tool MCP được đăng ký cùng registry với tool built-in và xuất hiện trong trace của desktop/CLI. Server khai báo capability `resources` hoặc `prompts` được thêm một tool chỉ đọc `mcp__<server>__extras` để liệt kê/đọc resources và prompts.

Có hai transport:

- **stdio**: spawn `command` + `args`, JSON-RPC theo dòng, protocol `2024-11-05`.
- **Streamable HTTP**: POST JSON-RPC tới `url`, nhận response JSON hoặc SSE, giữ `Mcp-Session-Id` do server trả về, protocol `2025-03-26`.

### Cấu hình `mcp.json`

```json
{
  "servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/ban/notes"],
      "env": {},
      "cwd": null,
      "enabled": true,
      "sandbox": {
        "enabled": true,
        "allow_network": false,
        "writable_paths": ["/home/ban/notes", "~/.npm"]
      }
    },
    "remote-docs": {
      "url": "https://mcp.example.com/mcp",
      "headers": { "Authorization": "Bearer ${TEZ_MCP_TOKEN}" },
      "enabled": true
    }
  }
}
```

| Trường | Ý nghĩa |
|---|---|
| `command` | Chương trình chạy server stdio, phải có trên `PATH`. Bỏ trống nếu dùng `url`. |
| `args` | Mảng tham số cho stdio. |
| `url` | Endpoint Streamable HTTP. Có `url` thì tez dùng HTTP thay vì spawn process. |
| `headers` | Header gửi kèm mỗi request HTTP, ví dụ `Authorization`; hỗ trợ `${VAR}`. Không nên đặt secret trực tiếp nếu file bị chia sẻ. |
| `bearer_token_env` | Tên biến môi trường chứa Bearer token; đọc lúc connect, không lưu token. |
| `env` | Biến môi trường bổ sung cho stdio. |
| `cwd` | Thư mục làm việc cho stdio; plugin mặc định dùng thư mục plugin. |
| `enabled` | `false` để tạm tắt mà không xóa cấu hình. |
| `sandbox` | Tùy chọn sandbox stdio trên Linux: `{ "enabled": true, "allow_network": false, "writable_paths": [...] }`. Bật thì cần `bubblewrap`; thiếu `bwrap` server bị từ chối, không chạy không sandbox. Mặc định chặn mạng, ẩn secret trong data dir và credential trong HOME; `writable_paths` được bind read-write (không được trỏ vào data dir). |
| `read_only` | `true` để ép mọi tool của server được coi là chỉ đọc. Alias khi đọc file: `readOnly`. |

Đường dẫn: `~/.local/share/teaching-ez/mcp.json`. Desktop ghi khóa này dưới dạng `read_only`; `readOnly` được chấp nhận khi bạn sửa file bằng tay.

### Chỉ đọc và Plan

Ngoài `read_only` ở cấp server, tool có thể tự khai báo chỉ đọc qua `annotations.readOnlyHint` (chuẩn MCP) hoặc `readOnly`/`read_only` ở cấp tool. Server `read_only` ghi đè mọi tool con thành chỉ đọc.

- **Plan** chỉ đăng ký các tool MCP được đánh dấu chỉ đọc; tool MCP thay đổi trạng thái không vào Plan.
- Permission policy cho qua tool read-only sau deny-list và rule `deny`, kể cả khi mode là `read_only`. Rule `ask`/`strict` không hỏi lại các tool này.
- App **tin vào khai báo của server**, không kiểm chứng hành vi thực tế. Một server khai báo sai vẫn có thể thay đổi dữ liệu; chỉ cài server bạn tin tưởng.
- Tool read-only có thể đọc tài nguyên ngoài workspace tùy server; Plan không sandbox MCP. Kể cả khi bật `sandbox`, server vẫn thấy filesystem gốc ở chế độ chỉ đọc (trừ data dir và credential đã ẩn), nên chỉ cài server bạn tin tưởng.

Desktop có checkbox **Chỉ đọc (cho phép trong Plan)** trong form MCP.

### Resources và prompts

Nếu server khai báo capability `resources` hoặc `prompts`, tez đăng ký thêm một tool chỉ đọc tên `mcp__<server>__extras`:

| `action` | Việc thực hiện |
|---|---|
| `list_resources` | Gọi `resources/list`, trả về danh sách resource. |
| `read_resource` | Gọi `resources/read` với `uri`; trả text hoặc mô tả blob. |
| `list_prompts` | Gọi `prompts/list`, trả về danh sách prompt. |
| `get_prompt` | Gọi `prompts/get` với `name` và `arguments` tùy chọn. |

Tool extras luôn được coi là chỉ đọc, nên dùng được trong Plan và không hỏi lại quyền. Nội dung trả về được giới hạn 64 KB. Đây vẫn là dữ liệu do server cung cấp; chỉ cài server bạn tin tưởng.

### Tên tool

Tool được đặt tên `mcp__<server>__<tool>` và làm sạch ký tự để tương thích OpenAI function calling, tối đa 64 ký tự. Ví dụ server `study`, tool `echo` → `mcp__study__echo`.

### OAuth cho MCP HTTP

Server Streamable HTTP có thể dùng OAuth 2.1 + PKCE. Khai báo trong `mcp.json`:

```json
{
  "servers": {
    "remote": {
      "url": "https://mcp.example.com/mcp",
      "oauth": {
        "authorization_url": "https://auth.example.com/authorize",
        "token_url": "https://auth.example.com/token",
        "client_id": "tez",
        "scopes": ["mcp:read", "mcp:write"]
      }
    }
  }
}
```

`client_secret` chỉ nên đặt qua biến môi trường và trỏ bằng `client_secret_env`. Luồng uỷ quyền dùng redirect loopback, kiểm tra `state`, lưu token tại `oauth.json` với quyền 0600 và tự refresh trước khi hết hạn. Trong **Cài đặt → MCP**, nút uỷ quyền hiện trạng thái và mở trình duyệt; token không được ghi vào `mcp.json` hay audit log.

### Giới hạn hiện tại

- HTTP chỉ dùng POST JSON-RPC + response JSON/SSE; chưa hỗ trợ GET stream server-initiated; header hỗ trợ `${VAR}` và `bearer_token_env`. Resources/prompts chỉ đọc qua tool extras, không có UI riêng.
- `mcp.json` hỏng không chặn phiên chat: server từ file đó bị bỏ qua kèm cảnh báo. Khi lưu từ Settings, file hỏng cũ được đổi tên thành `mcp.json.corrupt-<timestamp>-<pid>` trước khi ghi file mới atomically.
- Timeout khởi động/gọi: 20 giây; stderr của server stdio bị bỏ qua.
- Server stdio mặc định chạy dưới quyền user hiện tại; có thể bật sandbox bubblewrap theo từng server trên Linux. Khi bật: mạng bị chặn trừ khi `allow_network`, data dir và credential trong HOME bị ẩn, chỉ các đường dẫn trong `writable_paths` được ghi; thiếu `bwrap` thì server bị từ chối thay vì chạy không sandbox. macOS/Windows chưa có sandbox OS cho MCP. Chỉ cài server bạn tin tưởng. Process chỉ kế thừa các biến môi trường cơ bản (`PATH`, `HOME`, `LANG`, ...), không kế thừa API key/token của app; biến bổ sung phải khai báo trong `env`.
- Server stdio được spawn lại mỗi lượt chat; chưa giữ process dài hạn. HTTP dùng client mới mỗi lượt.

### CLI

```bash
tez mcp list
tez mcp test
tez mcp test study
```

---

## 3. Plugin — gói cấu hình

Plugin là một thư mục trong `~/.local/share/teaching-ez/plugins/<id>/` với `plugin.json`. Plugin có thể mang theo skill và MCP server, nhưng **không chạy code plugin tùy ý**. Bề mặt tin cậy chỉ gồm những MCP server được khai báo rõ ràng.

### Cấu trúc

```
plugins/study-toolkit/
  plugin.json
  server.py
  skills/
    exam-strategy/
      SKILL.md
```

### `plugin.json`

```json
{
  "id": "study-toolkit",
  "name": "Study Toolkit",
  "version": "0.1.0",
  "description": "Skill chiến lược phòng thi + MCP demo.",
  "skills": ["skills/exam-strategy"],
  "mcpServers": {
    "study": {
      "command": "python3",
      "args": ["server.py"],
      "enabled": true
    }
  }
}
```

- Đường dẫn trong `skills` là tương đối với thư mục plugin (hoặc tuyệt đối).
- MCP server từ plugin được thêm `cwd` = thư mục plugin nếu chưa khai báo.
- Server user tự thêm trong `mcp.json` được ưu tiên hơn server cùng tên từ plugin.
- Plugin không có bước build; sửa file rồi mở lại app.

### Plugin mẫu

Repo có sẵn `examples/plugins/study-toolkit/`:

- skill `exam-strategy` (chiến lược phòng thi),
- MCP server Python chỉ dùng thư viện chuẩn với tool `echo` và `add`.

Trong desktop: **Cài đặt → Plugin → Cài plugin mẫu**. Hoặc copy thủ công:

```bash
mkdir -p ~/.local/share/teaching-ez/plugins
cp -r examples/plugins/study-toolkit ~/.local/share/teaching-ez/plugins/
```

Sau đó kiểm tra:

```bash
tez plugins
tez mcp test study
tez skills show exam-strategy
```

### CLI

```bash
tez plugins
```

---

## 4. Extension theo dự án

Khi workspace được chọn (Chat hoặc Code), tez nạp thêm ngữ cảnh ngay trong dự án:

- `AGENTS.md`, `CLAUDE.md` ở gốc workspace, và bản trong `.tez/` — gộp vào system prompt dưới nhãn `<project-instructions>`, tối đa 24 KB.
- `.tez/skills/<id>/SKILL.md` — skill riêng của dự án; ghi đè skill cùng id từ user/plugin.
- `.tez/mcp.json` — MCP server riêng của dự án, cùng schema với `mcp.json` toàn cục (gồm cả `read_only`). Server dự án thắng server toàn cục khi trùng tên; `cwd` mặc định là gốc dự án.

Ưu tiên khi mâu thuẫn: yêu cầu trực tiếp của người dùng > chỉ dẫn dự án > skill/memory. Không cần bật Code mode để dùng chỉ dẫn dự án; chỉ cần chọn workspace ở sidebar.

---

## 5. Tạo extension bằng AI

Desktop có luồng **mô tả → xem trước → lưu** cho cả skill, MCP và plugin:

1. Mở `Ctrl K` → **Tạo Skill/MCP server/Plugin bằng AI**, hoặc nút **Tạo với AI** trong tab tương ứng.
2. Mô tả điều bạn muốn bằng ngôn ngữ tự nhiên.
3. AI trả về JSON gồm danh sách file; app hiện từng file để xem trước.
4. Bấm **Lưu vào máy** để ghi vào data dir.

Backend dùng hai lệnh `draft_extension` và `save_extension`. Bản nháp chỉ được ghi trong phạm vi:

- skill: `skills/<id>/SKILL.md`,
- plugin: `plugins/<id>/...`,
- MCP: thêm một entry vào `mcp.json`.

Đường dẫn tuyệt đối, `..` và định dạng file sai bị từ chối. Nội dung vẫn là file local; bạn có thể sửa tay sau khi lưu. AI không được cấp quyền chạy lệnh hay đọc dữ liệu ngoài data dir trong luồng này.

---

## 6. Thứ tự ưu tiên và an toàn

- Skill của user nằm trong `skills/`; skill từ plugin và skill dự án được nạp thêm. Khi trùng id: dự án > user > plugin.
- MCP server khi trùng tên: dự án > user > plugin.
- API key vẫn nằm trong vault AES-256-GCM; `mcp.json` chỉ chứa cấu hình, không chứa key provider.
- Nếu đưa `env` bí mật vào `mcp.json`, file được đặt quyền `0600` nhưng vẫn là plaintext trên đĩa. Ưu tiên biến môi trường của hệ điều hành khi có thể.
- `read_only`/`readOnlyHint` là khai báo của server, không phải sandbox hay xác minh quyền thực tế. Chỉ dùng trong Plan khi bạn tin server đó.
- Xóa `plugins/<id>/` là gỡ plugin; xóa `skills/<id>/` là gỡ skill.
