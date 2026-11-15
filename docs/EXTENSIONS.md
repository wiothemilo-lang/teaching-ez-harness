# Skills, MCP và plugin

> **Trạng thái:** MVP đã chạy trong CLI và desktop.
> **Nguyên tắc:** mọi thứ là file local, không cần build lại, không có marketplace hay code chạy ngầm ngoài MCP server do bạn khai báo.

---

## 1. Skills — prompt pack Markdown

Skill là một file `SKILL.md` chứa hướng dẫn cho agent. Khi người dùng hỏi, tez khớp **ngữ cảnh** (`subjects`) và **từ khóa** rồi chèn tối đa 2 skill vào system prompt của lượt đó. Skill không chạy code và không có quyền truy cập dữ liệu.

### Vị trí

```
~/.local/share/teaching-ez/skills/
  step-by-step/SKILL.md
  error-diagnosis/SKILL.md
  exam-review/SKILL.md
  english-writing/SKILL.md
```

Lần đầu mở app, bốn skill mặc định được tạo nếu thư mục chưa tồn tại.

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
| `triggers` | Từ khóa xuất hiện trong câu hỏi (không phân biệt hoa thường). Bỏ trống = luôn khớp ngữ cảnh. |
| `id` | Tùy chọn, dùng để đặt id ổn định. |

Phần thân sau `---` được render thành `<skill name="…">…</skill>` và cắt ở 4KB.

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

## 2. MCP — công cụ ngoài qua stdio

tez là MCP client tối giản: spawn server, gọi `initialize`, `tools/list`, `tools/call`, rồi tắt server khi lượt chat kết thúc. Tool MCP được đăng ký cùng registry với tool built-in và xuất hiện trong trace của desktop/CLI.

### Cấu hình `mcp.json`

```json
{
  "servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/ban/notes"],
      "env": {},
      "cwd": null,
      "enabled": true
    }
  }
}
```

| Trường | Ý nghĩa |
|---|---|
| `command` | Chương trình chạy server, phải có trên `PATH`. |
| `args` | Mảng tham số. |
| `env` | Biến môi trường bổ sung. |
| `cwd` | Thư mục làm việc; plugin mặc định dùng thư mục plugin. |
| `enabled` | `false` để tạm tắt mà không xóa cấu hình. |

Đường dẫn: `~/.local/share/teaching-ez/mcp.json`.

### Tên tool

Tool được đặt tên `mcp__<server>__<tool>` và làm sạch ký tự để tương thích OpenAI function calling, tối đa 64 ký tự. Ví dụ server `study`, tool `echo` → `mcp__study__echo`.

### Giới hạn hiện tại

- Chỉ transport **stdio**. HTTP/SSE chưa hỗ trợ.
- Timeout khởi động/gọi: 20 giây; stderr của server bị bỏ qua.
- Server chạy dưới quyền user hiện tại, **chưa có sandbox**. Chỉ cài server bạn tin tưởng.
- Server được spawn lại mỗi lượt chat; chưa giữ process dài hạn.

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
- `.tez/mcp.json` — MCP server riêng của dự án, cùng schema với `mcp.json` toàn cục. Server dự án thắng server toàn cục khi trùng tên; `cwd` mặc định là gốc dự án.

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
- Xóa `plugins/<id>/` là gỡ plugin; xóa `skills/<id>/` là gỡ skill.
