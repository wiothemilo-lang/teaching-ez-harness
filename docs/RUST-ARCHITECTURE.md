# Kiến trúc Rust + Tauri + Agent runtime

> **Trạng thái:** Draft v0.1
> **Quyết định:** Rust workspace, Tauri 2 cho desktop, CLI `tez`, local-first, BYOK.
> **Mục tiêu:** biến model của người dùng thành một **AI agent dạy học hoàn chỉnh** nhờ tools, context, memory và policy do harness kiểm soát.

---

## 1. Tư duy cốt lõi

Một chatbot chỉ có:

```
prompt → model → text
```

Một agent hoàn chỉnh có:

```
mục tiêu + context + tools + memory + policy
                    ↓
              agent loop
                    ↓
   model → tool call → kết quả → model → ... → câu trả lời
```

Harness của bạn cung cấp phần **không phải model**:

| Thành phần | Harness cung cấp |
|---|---|
| **Context** | Mục tiêu bài học, tài liệu RAG, hồ sơ học sinh, trạng thái phiên, lịch sử |
| **Tools** | Tính toán, vẽ đồ thị, cân bằng phương trình, tra từ, ghi evidence... |
| **Memory** | Phiên hiện tại, mastery dài hạn, misconception, lịch ôn |
| **Policy** | Socratic, scaffolding, không lộ đáp án, quyền tool theo độ tuổi |
| **Loop** | Gọi model → thực thi tool → quan sát → lặp → kết thúc |
| **Observability** | Trace từng bước, token, latency, tool call |

Model của người dùng chỉ cần làm phần **suy luận/ngôn ngữ**. Đó là lý do harness có giá trị.

---

## 2. Workspace Rust

```
Cargo.toml                     # workspace
crates/
  tez-core/                    # domain + agent runtime + context + project + tools
  tez-llm/                     # provider trait + OpenAI-compatible adapter + tool calls
  tez-auth/                    # connections, keychain, vault, OAuth
  tez-store/                   # SQLite, migrations, repositories
  tez-cli/                     # binary `tez`
  tez-update/                  # GitHub Releases check/download
apps/
  desktop/                     # React + Vite frontend
  desktop/src-tauri/           # Tauri shell (Rust), depends on tez-core
subjects/
  math/                        # manifest, prompts, rubrics, evals
  physics/
  chemistry/
  english/
docs/
```

**Vì sao workspace?** Desktop, CLI, server dùng chung core; Rust compile theo crate; test độc lập; sau này thêm crate `tez-sync` hoặc `tez-plugin`.

---

## 3. Agent runtime

### 3.1 Vòng lặp

```
1. Compactor chạy các bước rẻ trước khi gọi model:
   a. ghi kết quả tool lớn ra file, giữ preview;
   b. cắt bớt lịch sử cũ, lưu transcript;
   c. micro-compact kết quả cũ nếu vẫn quá giới hạn.
2. Nếu vẫn quá giới hạn → gọi model tóm tắt lịch sử (bước đắt, mất chi tiết).
3. Gọi provider với messages + tool schemas.
4. Nếu model trả text cuối:
   a. chạy hook `Stop` (có thể trả `continue` để agent làm tiếp);
   b. trả `AgentOutcome` và kết thúc.
5. Nếu model gọi tool, với từng call:
   a. chạy hook `PreToolUse` — có thể chặn thẳng;
   b. permission pipeline: deny-list → ask → allow;
   c. nếu `ask`, gọi `Approver` (dialog desktop hoặc terminal CLI; CLI in unified diff trước khi hỏi với `write_file`/`edit_file`);
   d. thực thi tool với timeout riêng của tool;
   e. cắt output theo `max_tool_output`;
   f. chạy hook `PostToolUse` — có thể thay output (ví dụ che secret);
   g. ghi tool result vào conversation.
6. Nếu agent có todo và lâu không cập nhật, chèn reminder `todo_write`.
7. Quay lại bước 1 (tối đa `max_steps`).
```

**Giới hạn mặc định (`AgentConfig` + `harness.json`):**

| Giới hạn | Giá trị |
|---|---|
| Số bước agent | 12 (chỉnh 1–100) |
| `max_tokens` mỗi phản hồi | 1024 |
| `temperature` | 0.2 |
| Timeout mỗi tool | 60s mặc định; `run_command` 130s; `task` 180s |
| Output tool tối đa | 8KB (cắt an toàn theo UTF-8) |
| Compaction | bật, ngưỡng 60.000 ký tự JSON |
| Token context | ước lượng bằng độ dài JSON messages, không cần tokenizer |

### 3.2 Kiểu dữ liệu

```rust
pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    config: AgentConfig,
    policy: Arc<PermissionPolicy>,
    approver: Arc<dyn Approver>,
    hooks: Arc<HookRegistry>,
    compactor: Compactor,
    todos: Option<Arc<TodoState>>,
}

pub struct RunContext<'a> {
    pub session_id: Option<&'a str>,
    pub artifact_dir: Option<&'a Path>,
    pub active_request: Option<&'a str>,
}

pub struct AgentOutcome {
    pub text: String,
    pub steps: Vec<AgentStep>,
    pub usage: Usage,
    pub todos: Vec<TodoItem>,
    pub compactions: CompactReport,
    pub permission_denials: usize,
    pub stop_reason: StopReason, // Completed | MaxSteps | Stopped
}
```

Agent được lắp bằng builder: `Agent::new(provider, tools)` rồi
`.with_config(...)`, `.with_policy(...)`, `.with_approver(...)`,
`.with_hooks(...)`, `.with_compactor(...)`, `.with_todos(...)`.
`run_with(model, messages, RunContext)` là entry point chính; `run()` chỉ là
wrapper không có session/artifact.

### 3.3 Streaming

Provider phát text theo từng đoạn qua
`LlmProvider::chat_stream(req, Option<UnboundedSender<String>>)`.
`OpenAiCompatible` gửi `stream: true`, đọc SSE `data:` cho đến `[DONE]`, đẩy
`delta.content` vào sender và gom `delta.tool_calls` theo `index` thành
`ToolCall` hoàn chỉnh. Provider không hỗ trợ streaming dùng default method:
gọi `chat` rồi gửi toàn bộ text một lần.

Desktop nối sender đó vào event `tez://chat-delta` (`{ session_id, text }`);
frontend ghép dần vào một message tạm, rồi thay bằng message cuối khi lượt chạy
xong. CLI dùng cùng sender qua `with_stream` và một task `spawn_blocking` in từng delta ra stdout; tool trace và thống kê vẫn đi stderr. Permission, hook và compaction không phụ thuộc vào
cách provider trả dữ liệu.

### 3.4 Kết thúc

Agent dừng khi:

- Model trả câu trả lời cuối và hook `Stop` không yêu cầu tiếp.
- Chạm `max_steps` → `StopReason::MaxSteps`, kèm text gợi ý chia nhỏ việc.
- Người dùng bấm dừng → `StopReason::Stopped` với outcome một phần (text và
  tool đã chạy vẫn được giữ).
- Lỗi không phục hồi → `AgentError`.

---

## 4. Context builder — phần quan trọng nhất

### 4.1 Nguồn context

| Nguồn | Nội dung | Ưu tiên |
|---|---|---|
| **System** | Vai trò chat/code, ngôn ngữ, chỉ dẫn bổ sung của người dùng | Pinned |
| **Project** | `AGENTS.md`/`CLAUDE.md` và `.tez/` của workspace, render thành `<project-instructions>`, tối đa 24 KB | Cao |
| **Skills** | Catalog rút gọn theo nguồn (tối đa 12 id/nguồn); tool `skill` không tên trả danh mục đầy đủ, có tên trả body | Cao |
| **Memory** | Memory recall theo từ khóa | Cao |
| **Todos** | Kế hoạch hiện tại của phiên | Pinned |
| **RAG** | Top-k chunk tài liệu liên quan, kèm citation | Cao |
| **Summary** | Tóm tắt các lượt cũ khi compaction | Trung bình |
| **Recent turns** | Lượt gần nhất nguyên văn (CLI mặc định 12 message, desktop tối đa 40); lượt hiện tại có thể kèm ảnh, và tối đa 2 lượt user gần nhất được gửi lại ảnh trong ngân sách 6 ảnh/8 MB | Cao |
| **Tool specs** | Schema các tool được phép | Pinned |

### 4.2 Token budget

```rust
pub struct BudgetPolicy {
    pub total: usize,          // theo model
    pub system_pct: u8,        // 10
    pub instructions_pct: u8,  // 10
    pub retrieval_pct: u8,     // 30
    pub memory_pct: u8,        // 20
    pub conversation_pct: u8,  // 30
}
```

- Pinned sections luôn giữ.
- Flexible sections bị cắt/tóm tắt theo priority.
- Nếu vẫn quá budget: giảm RAG top-k, rút gọn recent turns, thay bằng summary.
- Tokenizer: `tiktoken-rs` cho OpenAI; xấp xỉ cho provider khác (dùng metadata model).

### 4.3 Compaction

Compaction là pipeline bốn bước, chạy từ rẻ tới đắt
(`crates/tez-core/src/compaction.rs`):

1. **Persist kết quả tool lớn** — kết quả > `large_result_chars` được ghi vào
   `sessions/<id>/tool-results/<call_id>.txt`; conversation chỉ giữ preview.
2. **Snip lịch sử** — khi số message vượt `trigger_messages`, lưu toàn bộ
   transcript vào `sessions/<id>/transcripts/<timestamp>.json`, giữ vài message
   đầu + `target_messages` message gần nhất, chèn marker có đường dẫn transcript.
3. **Micro-compact** — nếu vẫn vượt `context_char_limit`, thay kết quả tool cũ
   bằng đường dẫn file, giữ `keep_recent_results` kết quả gần nhất.
4. **LLM summary** — chỉ khi vẫn vượt giới hạn: gọi model tóm tắt mục tiêu,
   file đã đọc/sửa, quyết định, việc còn lại; giữ system prompt và yêu cầu hiện
   tại, kèm đường dẫn transcript.

Thứ tự này cố định vì bước 1–3 không gọi model và luôn có đường phục hồi; bước
4 mất chi tiết nên chỉ là phương án cuối. Nếu provider vẫn báo lỗi context
(`prompt is too long`, `maximum context`), agent thử tóm tắt một lần rồi chạy
lại. Agent cũng có tool `compact` để chủ động nén sau một giai đoạn dài.

`harness.json` chỉnh được `enabled`, `context_char_limit`, `large_result_chars`,
`preview_chars`, `trigger_messages`, `target_messages`, `keep_recent_results`.

### 4.4 Context inspector

Đây là tính năng debug quan trọng:

- `tez context inspect --session <id>` in ra từng section, token count, phần bị cắt.
- Desktop có panel "Context" xem section nào được gửi.
- Giúp hiểu tại sao model trả lời sai (thiếu tài liệu? thiếu state?).

---

## 5. Tools

### 5.1 Trait

```rust
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec; // name, description, JSON schema
    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
    async fn execute(
        &self,
        args: serde_json::Value,
    ) -> Result<ToolOutput, ToolError>;
}

pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}
```

Tool không tự quyết định permission; policy tập trung ở `Agent`. Tool chỉ khai
báo schema và (nếu cần) timeout dài hơn mặc định.

### 5.2 Registry, permission & hook

- **Registry**: `ToolRegistry` giữ tool built-in, tool MCP (`mcp__server__tool`)
  và tool harness (`todo_write`, `task`, `memory_*`, `compact`). Mỗi subject
  pack lọc allowlist; chế độ Code dùng `code_registry(root)`; chế độ Plan dùng
  `PLAN_TOOLS` + `skill`/`web_fetch` và chỉ thêm tool MCP được khai báo
  read-only.
- **Skill tool** (`tools/skill.rs`): `name` là tùy chọn. Không có `name`, tool
  trả danh mục đầy đủ nhóm theo nguồn (`catalog_grouped`, tối đa 200 skill và
  24.000 ký tự). Có `name`, tool tìm theo id rồi tên và trả body trong thẻ
  `<skill>` (cap 12.000 ký tự). System prompt chỉ nhận `catalog_overview`:
  mỗi nguồn một dòng, tối đa 12 id tiêu biểu. Skill dự án đứng trước
  user/plugin/built-in; đây là progressive disclosure hai tầng.
- **MCP** (`mcp.rs`): `McpTransport` trừu tượng hóa stdio và Streamable HTTP.
  Stdio spawn process, giao tiếp JSON-RPC theo dòng, protocol `2024-11-05`.
  HTTP dùng POST JSON-RPC tới `url`, gửi `Accept: application/json,
  text/event-stream`, lưu `Mcp-Session-Id` từ response và gửi lại ở các request
  sau, đọc response JSON hoặc SSE (`data:` event), protocol `2025-03-26`;
  `headers` cấu hình được dùng cho auth tĩnh. Cả hai đều gọi `initialize`,
  `notifications/initialized`, `tools/list`; `tools/call` đi qua cùng trait.
  Nếu server khai báo capability `resources`/`prompts`, discovery tạo thêm tool
  read-only `mcp__<server>__extras` với action `list_resources`, `read_resource`,
  `list_prompts`, `get_prompt`. Chưa hỗ trợ GET stream server-initiated hay
  OAuth; auth HTTP dùng header tĩnh.
- **Web fetch** (`tools/web.rs`): chỉ http/https, chặn userinfo, localhost và IP
  nội bộ (kể cả CGNAT, dải tài liệu IPv6, 6to4). Ở chế độ static, mỗi redirect
  được resolve DNS lại, kiểm tra toàn bộ địa chỉ rồi pin bằng
  `resolve_to_addrs`; timeout 20 s, tối đa 5 redirect, body static cap 512 KB có
  đánh dấu khi cắt, charset UTF-8/Latin-1/Windows-1252, lọc HTML và giải mã
  entity số. Ở static, content
  type `application/pdf`, `audio/*`, `video/*` bị từ chối rõ ràng. Chế độ
  `render: "browser"` chạy Chromium/Chrome/Edge headless tùy chọn (`TEZ_CHROMIUM`, hoặc
  `chromium`/`chrome`/`msedge` trên `PATH`) để lấy DOM sau JavaScript, timeout
  25 s; nếu thiếu browser hoặc browser lỗi, tool tự fallback sang static và ghi
  rõ lý do. `--no-sandbox` chỉ bật khi `TEZ_BROWSER_NO_SANDBOX=1`. Browser map
  toàn bộ địa chỉ qua `--host-resolver-rules` nhưng chỉ kiểm tra URL/host ban
  đầu, không revalidate từng redirect. Policy mặc định hỏi trước `web_fetch` vì đây
  là truy cập mạng ngoài.
- **Permission pipeline** (`crates/tez-core/src/permission.rs`) có bốn mode:
  `read_only`, `strict`, `standard` (mặc định), `trust`. Mỗi call đi qua:
  deny-list (lệnh phá hủy) → rule `deny` của người dùng → tool MCP read-only
  được cho qua → chặn `read_only` → rule `allow` → ask mặc định (xóa,
  `git reset --hard`, cài package, `curl | sh`, tool MCP…) → rule `ask` →
  `strict`. Rule lưu trong `harness.json` dưới dạng `{action, tool, pattern}`;
  `tool` nhận `*`, pattern là glob khớp path (tool file) hoặc command
  (`run_command`). Deny-list lệnh phá hủy luôn thắng rule, và `read_only` không
  bị rule `allow` vượt qua; tool MCP read-only cũng không vượt được deny-list
  hay rule `deny`.
- **Approver**: trait async. Desktop hiển thị dialog và trả lời qua event
  `tez://approval-request` + command `respond_approval({ id, approved, remember })`;
  với `write_file` và `edit_file`, desktop đọc file cũ trong
  workspace và gửi kèm unified diff ở `preview` để người dùng duyệt trước khi
  ghi. Checkbox **Cho phép thao tác này trong phiên hiện tại** khiến broker ghi
  khóa `session_id|tool|path/command|fingerprint(args)` vào
  `approvals.json` trong data dir (mặc định 7 ngày/200 mục, chỉnh trong Agent,
  file 0600, ghi atomic qua file tạm rồi rename) và tự duyệt các request khớp sau đó, kể cả sau
  khi khởi động lại; file corrupt bị bỏ qua và ghi log. CLI dùng
  `dialoguer::Confirm`, in unified diff trước prompt cho `write_file`/
  `edit_file` (cap 24.000 ký tự), có `--yes` để tự duyệt và `--permission` để
  ghi đè mode.
- **Hooks** (`crates/tez-core/src/hooks.rs`): `UserPromptSubmit`, `PreToolUse`,
  `PostToolUse`, `Stop`. Hook built-in ghi audit JSONL
  (`<data_dir>/audit.jsonl`) và che API key/private key trong output tool.
  Người dùng thêm hook shell trong `harness.json`; hook nhận JSON qua stdin và
  trả `{"block": "...", "output": "...", "continue": "...", "note": "..."}`.
- **Subagent**: tool `task` chạy một `Agent` mới với `messages[]` sạch, cùng
  workspace/policy/hook, không có tool `task` (một tầng delegation). Chỉ văn bản
  cuối quay về agent cha.
- Tool args được validate theo JSON Schema ở tầng tool; tool result luôn là
  **dữ liệu không tin cậy**, không thực thi chỉ dẫn nằm trong đó.

### 5.3 Danh mục tool

#### Dùng chung

| Tool | Quyền | MVP | Mô tả |
|---|---|---|---|
| `search_curriculum(query, lesson_id)` | ReadOnly | ✅ | RAG trên tài liệu bài học, trả citation |
| `get_lesson_context(lesson_id)` | ReadOnly | ✅ | Mục tiêu, rubric, tài liệu |
| `get_student_profile()` | ReadOnly | ✅ | Mastery, misconception, hint level |
| `record_evidence(objective_id, correct, hint_level)` | WriteStudentData | ✅ | Cập nhật mastery |
| `generate_practice_item(objective_id, difficulty)` | ReadOnly | ✅ | Sinh câu luyện (structured output) |
| `escalate_to_teacher(reason)` | WriteStudentData | ✅ | Đánh dấu cần giáo viên |
| `cite_source(source_id)` | ReadOnly | ✅ | Trích dẫn nguồn |
| `create_flashcard(front, back)` | WriteStudentData | Sau | Tạo thẻ ôn |
| `schedule_review(objective_id, date)` | WriteStudentData | Sau | Lịch ôn |
| `grade_response(rubric, response)` | TeacherOnly | Sau | Chấm nháp, giáo viên duyệt |

#### Toán

| Tool | MVP | Crate/approach |
|---|---|---|
| `calculate(expression)` | ✅ | `evalexpr` hoặc `meval` (không eval code) |
| `plot_function(expression, x_min, x_max)` | ✅ | Trả data/SVG; frontend vẽ |
| `check_step(problem, step)` | ✅ | So sánh numeric; symbolic hạn chế |
| `solve_equation(equation, variable)` | Sau | CAS/SymPy sidecar nếu cần |

#### Lý

| Tool | MVP | Crate/approach |
|---|---|---|
| `convert_units(value, from, to)` | ✅ | Bảng đơn vị + hệ số |
| `physics_formula(name, vars)` | ✅ | Formula registry (JSON/TOML) |
| `calculate_with_units(expression)` | Sau | Unit-aware evaluator |

#### Hóa

| Tool | MVP | Crate/approach |
|---|---|---|
| `balance_equation(equation)` | ✅ | `nalgebra` giải hệ nguyên tố |
| `molar_mass(formula)` | ✅ | Parser công thức + bảng tuần hoàn |
| `stoichiometry(equation, given, target)` | ✅ | Tỉ lệ mol |
| `periodic_table(element)` | ✅ | JSON dữ liệu |

#### Anh

| Tool | MVP | Crate/approach |
|---|---|---|
| `check_grammar(text)` | ✅ | `harper` (nếu ổn) hoặc model-based |
| `readability(text)` | ✅ | Flesch-Kincaid |
| `vocabulary_lookup(word)` | ✅ | Dictionary JSON + RAG |
| `grade_writing(essay, rubric)` | Sau | Model + rubric, giáo viên duyệt |

### 5.4 Model không hỗ trợ tool calling

Không phải model nào cũng có native tool calls (nhất là Ollama nhỏ).

**Fallback protocol:**

1. Đưa schema tool vào system prompt.
2. Yêu cầu model trả JSON:
   ```json
   {"type":"tool_call","name":"calculate","args":{"expression":"2+2"}}
   ```
3. Parse + validate; nếu sai → trả lỗi cho model thử lại (tối đa 2 lần).
4. Nếu vẫn sai → trả lời không dùng tool.

Gateway khai báo capability `tool_calling: native | prompted | none`.

---

### 5.5 Chế độ Code — workspace tools

`crates/tez-core/src/tools/workspace.rs` cung cấp các tool gắn với một
`Workspace` (root đã canonicalize):

| Tool | Việc |
|---|---|
| `read_file` | đọc text ≤ 256 KB; `offset`/`limit` để đọc theo dòng |
| `write_file` | ghi đè toàn bộ nội dung, tạo thư mục cha |
| `edit_file` | thay `old_string` bằng `new_string`; mặc định phải khớp duy nhất, `replace_all` để thay mọi lần |
| `list_files` | liệt kê ≤ 300 entry, 1–4 tầng, bỏ `node_modules`/`target`/`.git` |
| `glob` | tìm file theo pattern `src/**/*.rs`, bỏ thư mục build |
| `grep` | tìm regex trong file text; lọc `include`, `ignore_case`, trả `path:dòng: nội_dung` |
| `run_command` | `sh -c` (`cmd /C` trên Windows), timeout 130 s, cwd trong workspace |
| `compact` | agent chủ động yêu cầu nén lịch sử sau một giai đoạn |

`Workspace::resolve` ghép path tương đối với root, chuẩn hóa `.`/`..`, rồi
canonicalize phần đã tồn tại để chặn symlink trỏ ra ngoài. Đây là giới hạn ở
tầng ứng dụng, không phải sandbox OS. `tools::code_registry(root)` đăng ký các
tool file/lệnh; `CODE_TOOL_NAMES` là danh sách chuẩn. `Mode::Code` trong
`crates/tez-core/src/modes.rs` chọn prompt system gắn workspace và allowlist
`CODE_TOOLS`, bao gồm cả `todo_write`, `task`, `compact` và `memory_*`.

### 5.6 Chế độ Plan

`Mode::Plan` dùng cùng workspace với Code nhưng chỉ đăng ký `PLAN_TOOLS`
(`read_file`, `list_files`, `glob`, `grep`, `todo_write`, `memory_search`),
cộng thêm `skill`, `web_fetch` (mặc định vẫn hỏi duyệt vì truy cập mạng) và các
tool MCP được khai báo read-only. Tool ghi, chạy lệnh, `task`, `compact`,
`memory_save`/`memory_forget` và MCP thay đổi trạng thái không vào Plan. Policy
bị ép về `read_only` bất kể cấu hình, nên kể cả rule `allow` cũng không mở được
thao tác thay đổi; riêng tool MCP read-only được policy cho qua sau deny-list và
rule `deny`. Prompt `plan_system_prompt` yêu cầu agent khảo sát rồi trình kế
hoạch đúng schema Markdown 6 mục cấp 2: `Mục tiêu`, `Phạm vi`,
`File và thay đổi`, `Các bước`, `Rủi ro`, `Kiểm chứng`, rồi dừng để người dùng
chuyển sang Code. `plan.rs` parse heading cấp 2 (heading cấp 3 vẫn thuộc mục
cha), coi mục thiếu hoặc rỗng là chưa hoàn chỉnh; PlanDialog có form nhập từng
mục bên cạnh Markdown đầy đủ, hiện cảnh báo mục thiếu; **Lưu kế hoạch** từ chối
bản chưa đủ, **Lưu nháp** cho phép giữ bản chưa hoàn chỉnh; CLI in cảnh báo ra stderr
nhưng vẫn lưu để người dùng bổ sung.

Mỗi lượt Plan, văn bản cuối được lưu vào `plans/<session>.md` qua
`crates/tez-core/src/plan.rs`; file bị cap 200.000 ký tự và tên session được làm
sạch (chỉ ASCII chữ/số, `-`, `_`) để không thoát khỏi thư mục `plans/`. Desktop
dùng chung `ChatView` với cờ `planMode`; nút **Kế hoạch** trên header mở
`PlanDialog`, gọi `load_session_plan`/`save_session_plan` để xem, sửa và lưu plan;
`save_session_plan` nhận `allow_incomplete` — **Lưu kế hoạch** gửi `false` và bị
từ chối nếu thiếu mục, **Lưu nháp** gửi `true`
mà không cần chạy lại agent. CLI nhận `--mode plan` và cũng lưu plan.
Khi chuyển Plan → Code, desktop giữ nguyên `session_id`, cập nhật mode trong
SQLite (`Store::update_session_mode`), nạp lại plan đã lưu và chèn chỉ dẫn “thực
thi theo kế hoạch” vào system prompt; todos và lịch sử vẫn được nạp lại như một
phiên bình thường. CLI nạp plan đã lưu ở mỗi lượt Code (tối đa 20.000 ký tự khi
chèn), không chỉ ở lần chuyển mode.

## 6. Subject packs

Mỗi môn là một gói cấu hình + code tool, không phải app riêng.

> **MVP hiện tại:** pack được compile-time trong `crates/tez-core/src/subjects.rs`
> (`SubjectPack` gồm id, tên, alias, strategy, prompt, allowlist tool). Bản load
> `subject.toml` từ thư mục `subjects/` sẽ thêm khi cần giáo viên sửa không build lại.

> **Pack `general` là mặc định.** Nó không gắn với môn nào, chỉ có prompt trợ lý
> tổng quát + tool `calculate`. `subjects::resolve_or_general` không bao giờ lỗi:
> chuỗi rỗng hoặc id lạ đều rơi về `general`, nên chat tự do không bị chặn bởi
> danh sách môn. Bốn pack môn học chỉ còn là preset tùy chọn.

```
subjects/math/
  subject.toml
  prompts/tutor.md
  prompts/planner.md
  rubrics/problem_solving.toml
  formulas/          # nếu có
  evals/*.json
```

`subject.toml`:

```toml
id = "math"
name = "Toán"
grade_levels = ["6-12"]
language = "vi"
tools = ["calculate", "plot_function", "check_step", "search_curriculum", "record_evidence"]
strategy = "socratic"
```

### 6.1 Bốn môn, nhưng không làm sâu cả bốn cùng lúc

| Môn | Trọng tâm MVP | Tool MVP | Eval |
|---|---|---|---|
| **Toán** | Giải thích từng bước, Socratic | calculate, plot, check_step | 8 case |
| **Lý** | Công thức, đơn vị, bài tập định lượng | convert_units, physics_formula, calculate | 8 case |
| **Hóa** | Cân bằng phương trình, mol, stoichiometry | balance_equation, molar_mass, stoichiometry | 8 case |
| **Anh** | Ngữ pháp, viết, từ vựng | check_grammar, readability, vocabulary_lookup | 8 case |

**Chiến lược:** dựng engine + 4 pack mỏng trước; làm sâu **Toán** đầu tiên vì tool phong phú và dễ eval. Sau đó Hóa/Lý (dùng chung calculator), rồi Anh.

### 6.2 Agent profiles

| Profile | Dùng cho | Tools | Ghi chú |
|---|---|---|---|
| `TutorAgent` | Học sinh | Read-only + record_evidence | MVP |
| `PlannerAgent` | Giáo viên soạn bài | search, get_lesson, generate_practice_item | MVP |
| `GraderAgent` | Chấm nháp | rubric, search, grade_response | Sau |
| `ContentAgent` | Sinh nội dung | search, generate_practice_item | Sau |

Cùng một agent loop; khác prompt, tool allowlist, policy.

---

## 7. Provider layer

**MVP: OpenAI-compatible adapter** phủ được:

- OpenAI
- Groq
- OpenRouter
- Cloudflare Workers AI (OpenAI-compatible endpoint)
- Ollama / LM Studio / vLLM
- Custom gateway

**Sau:** Anthropic native, Gemini native (hoặc dùng endpoint OpenAI-compatible của Gemini), Vertex, Bedrock.

`LlmProvider` trait:

```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
    async fn chat_stream(
        &self,
        req: ChatRequest,
        tx: Option<UnboundedSender<String>>,
    ) -> Result<ChatResponse, LlmError>;
}
```

- `ChatRequest`: model, messages, tools, temperature, top_p, max_tokens.
- `Message` có `content: Option<String>` và `images: Vec<String>`; khi có ảnh,
  `message_json` gửi content dạng array `text` + `image_url` (data URL hoặc URL
  công khai). Không có ảnh thì wire format giữ nguyên như cũ. `tez-store` ghi ảnh
  thành file trong `images/<session>/<message>-<index>.<ext>` và chỉ giữ ref
  `tez-image:` trong cột `messages.images`; `load_messages` giải mã ref thành data
  URL khi resume. Khi dựng history, desktop chỉ gửi lại ảnh của tối đa 2 lượt user
  gần nhất; ngân sách của cả request là 6 ảnh và 8 MB, ưu tiên ảnh của lượt hiện
  tại (tối đa 4 ảnh).
- `ChatResponse`: text hoặc tool calls + usage.
- `chat_stream`: phát text delta qua `tx`, trả `ChatResponse` cuối; tool call
  được gom đủ theo `index` trước khi trả.
- Chưa có capability descriptor; mỗi provider tự xử lý phần nó hỗ trợ.

Chi tiết auth/BYOK: [PROVIDER-AUTH.md](PROVIDER-AUTH.md).

---

## 8. Storage

Schema hiện tại là version 5. `Store::open` chạy `SCHEMA` idempotent rồi thêm cột
còn thiếu (`sessions.subject`, `sessions.mode`) để nâng cấp DB cũ.

```sql
profiles (id, name, age_mode, created_at)
connections (id, profile_id, provider_id, label, auth_type,
             encrypted_payload, metadata, status, last_used_at, created_at)
lessons (id, profile_id, title, subject, grade_level, objectives,
         strategy, created_at)
sessions (id, lesson_id, profile_id, subject, mode, state, summary,
          started_at, ended_at)
messages (id, session_id, role, content, model, tokens_in, tokens_out, created_at)
tool_calls (id, session_id, tool_name, args_json, output, ok,
            duration_ms, created_at)
session_todos (session_id, items_json, updated_at)
context_snapshots (id, session_id, step_no, sections_json, tokens_json, created_at)
ai_usage (id, profile_id, connection_id, provider_id, model_id, task,
          tokens_in, tokens_out, latency_ms, status, created_at)
documents (id, profile_id, subject, title, source_path, mime, bytes, created_at)
document_chunks (id, document_id, chunk_no, content, created_at)
document_chunks_fts (FTS5: content, search_text, chunk_id, document_id)
```

- `session_todos` giữ kế hoạch của phiên dưới dạng JSON array; desktop/CLI nạp
  lại khi resume và ghi sau mỗi lượt.
- `context_snapshots` giữ chỗ cho context inspector; bản hiện tại chưa ghi.
- Artifact compaction nằm ngoài DB: `sessions/<id>/tool-results/*.txt` và
  `sessions/<id>/transcripts/*.json`. Plan của phiên nằm ở `plans/<session>.md`
  (cap 200.000 ký tự, tên file đã làm sạch).
- Không lưu secret; tool args có thể chứa dữ liệu học sinh → retention ngắn,
  cho phép xóa.
- SQLite WAL + `busy_timeout=5000`; desktop và CLI có thể mở đồng thời, tranh
  chấp ngắn được SQLite xử lý.

---

## 9. Local server API (kế hoạch)

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
POST /v1/agent/chat          # SSE: delta, tool_call, tool_result, state, usage, done
GET  /v1/sessions
GET  /v1/sessions/:id
GET  /v1/sessions/:id/trace
POST /v1/context/inspect     # trả context plan (không gọi model)
GET  /v1/usage
```

- Bind `127.0.0.1`, bearer token từ `server.json`.
- SSE event format:
  ```
  event: delta
  data: {"text":"..."}

  event: tool_call
  data: {"id":"...","name":"calculate","args":{...}}
  ```
- Không trả secret.

---

## 10. Tauri desktop

- Tauri 2, React + Vite; giao diện dark kiểu Claude Code.
- **Không có local HTTP server trong bản hiện tại.** Frontend gọi trực tiếp
  Tauri commands (in-process), không qua axum/SSE.
- Commands chính:
  - `app_status`, `list_sessions`, `load_session`, `chat`, `stop_chat`;
  - `load_session_plan`, `save_session_plan`;
  - `respond_approval`, `clear_approvals`, `get_harness`, `save_harness`, `delete_memory`;
  - `list_providers`, `save_provider`, `delete_provider`, `activate_provider`,
    `test_provider`, `test_connection`;
  - `list_extensions`, `save_mcp_server`, `delete_mcp_server`, `test_mcp_server`,
    `install_sample_plugin`, `open_extension_path`;
  - `check_update`, `download_update`, `launch_update`, `open_external`,
    `open_workspace`.
- **Approval flow:** agent gặp call cần duyệt → `DesktopApprover` emit event
  `tez://approval-request` kèm `ApprovalRequest { id, tool, args, reason,
  session_id }` → React mở modal → `respond_approval({ id, approved, remember })`
  → oneshot trả kết quả cho agent. Nếu `remember` và được duyệt, broker ghi
  `session_id|tool|path/command|fingerprint(args)` vào `approvals.json` (mặc định
  TTL 7 ngày/200 mục; chỉnh trong Agent) và tự duyệt các request khớp sau đó, kể
  cả sau khi khởi động lại; args khác thì hỏi lại. `clear_approvals` xoá toàn bộ
  approval đã nhớ.
  Timeout 180 s thì từ chối.
- `ChatReply` trả thêm `todos`, `compactions`, `permission_denials`,
  `stop_reason`; `SessionDetail` trả `todos` đã lưu.
- Màn hình hiện tại:
  - **Chat** — trò chuyện tổng quát, không bắt chọn môn.
  - **Plan** — chỉ đọc workspace, lưu plan vào `plans/<session>.md`.
  - **Code** — workspace tools, tool trace, todo/context dock, approval modal.
  - **Settings** — Providers, Skills, MCP, Plugins, **Harness**, Update, Data.
  - **Harness** — permission mode, permission rule (validate từng dòng, tắt
    **Lưu** khi lỗi), giới hạn agent, compaction, memory + xoá bản ghi, hook
    shell; hiển thị đường dẫn `harness.json`/memory/audit.
- Tool call card: tên, args, kết quả, thời gian, trạng thái ok/error.
- Security: Tauri capabilities allowlist, CSP, không load remote content.

Chi tiết đóng gói: [DESKTOP-CLI.md](DESKTOP-CLI.md).

---

## 11. CLI `tez`

```bash
tez connect --provider groq
tez connect --provider ollama
tez chat "Giải thích borrow checker"
tez chat --mode code --workspace ~/projects/app "Sửa lỗi build"
tez chat --session <id> --history 20 "Tiếp tục việc còn dở"
tez chat --yes --permission trust --max-steps 20 "Chạy test và sửa lỗi"
tez chat --permission read_only "Đọc và giải thích kiến trúc repo"
tez chat --no-memory "Bỏ qua memory đã lưu"
tez tools list
tez tools test calculate --args '{"expression":"2+2"}'
tez subject list
tez sessions
tez memory list
tez memory search "quy ước build"
tez memory forget "Build command"
tez harness
tez usage
tez doctor
tez update --check-only
```

- `tez chat` không có prompt và không có stdin pipe thì vào chế độ chat
  tương tác; có pipe thì đọc một lần từ stdin.
- `--mode code` dùng workspace tools; `--mode plan` chỉ đọc và lưu kế hoạch vào
  `plans/<session>.md`; `--workspace` mặc định là thư mục hiện tại. `--session`
  khôi phục mode/môn học đã lưu của session.
- `--yes` tự duyệt mọi thao tác cần permission (CI/script); `--permission`
  ghi đè `harness.json`; `--max-steps` và `--no-memory` ghi đè giới hạn và
  auto-recall cho riêng lượt chạy. Khi hỏi duyệt `write_file`/`edit_file`, CLI
  in unified diff (cap 24.000 ký tự) trước prompt lý do/tool/args.
- `tez memory list|search|forget` đọc/xoá bản ghi Markdown trong
  `<data_dir>/memory`; `tez harness` in permission mode, giới hạn agent,
  compaction, memory, hook và đường dẫn audit.
- Text được stream ra stdout trong lúc agent chạy; tool trace, cảnh báo permission
  và thống kê nén đi ra stderr để pipe stdout vẫn sạch. Desktop stream qua event
  `tez://chat-delta`, CLI dùng `with_stream` + task in delta.
- Lỗi trả exit code 1 kèm thông báo; `tez doctor` kiểm tra data dir, provider,
  DB, skill, MCP, plugin, harness và dung lượng.

---

## 12. Đóng gói & CI

- `tauri build` cho desktop: deb/rpm/AppImage, dmg, msi/nsis.
- `cargo build --release -p tez-cli`; phát hành binary qua GitHub Releases.
- `cargo dist` (tùy chọn) cho CLI đa nền tảng.
- GitHub Actions matrix: Linux/macOS/Windows; cache cargo; build Tauri cần system deps.
- Code signing: bỏ qua giai đoạn đầu; ghi rõ cách mở installer unsigned.
- Update: crate `tez-update` gọi GitHub Releases API (repo mặc định
  `teaching-ez/tez`, ghi đè bằng `TEZ_GITHUB_REPO`), tải asset vào
  `<data_dir>/updates`; không tự cài. Desktop hiển thị panel cập nhật; CLI dùng
  `tez update --check-only` hoặc `--download`.
- Windows: script PowerShell trong `scripts/windows/` và installer NSIS
  `currentUser`; workflow release tạo asset khi push tag `v*`.

---

## 13. Bảo mật

| Rủi ro | Biện pháp |
|---|---|
| Tool nguy hiểm | Code mode có `run_command` nhưng bị giới hạn trong workspace ở tầng ứng dụng, qua permission pipeline (deny → ask → allow) và audit/redaction; đây không phải sandbox OS |
| MCP read-only trong Plan | Chỉ tool được server khai báo `read_only`/`readOnlyHint` mới vào Plan; tool `mcp__<server>__extras` (resources/prompts) luôn read-only. App tin khai báo, không xác minh hành vi. Transport: stdio + Streamable HTTP (JSON/SSE, session header, headers tĩnh) |
| Prompt injection qua tài liệu/tool result | Coi là data; delimiter; không thực thi chỉ dẫn; tool allowlist |
| Key rò rỉ | Keychain + AES-GCM; redaction; không trả secret |
| Local server bị gọi trái phép | 127.0.0.1 + bearer token + Origin check |
| Ghi dữ liệu sai | Write tool cần confirmation; audit; undo nếu có thể |
| Lộ đáp án | Policy Socratic + kiểm tra hint level; eval |
| Dữ liệu trẻ em | Local-only; age mode; không telemetry mặc định |
| OAuth consumer | Experimental, cảnh báo ToS; API key là lõi |

---

## 14. Testing & eval

- **Unit:** tool math/chemistry/workspace; permission (deny/ask/allow, trust,
  read-only, MCP read-only); compaction 4 bước; memory lưu/tìm/xoá; todo; plan
  save/load + schema 6 mục + cap + sanitize tên file; hook block + redaction;
  harness config sanitize; approval persistence TTL/corrupt file; store
  migration v5, `session_todos`, image file refs; web_fetch parser, DNS
  pinning, browser resolver/fallback; MCP HTTP config, JSON/SSE parsing,
  session header and resources/prompts formatting; approval fingerprint.
- **Integration:** mock provider + agent loop; tool call → result → final;
  permission denial; subagent trả text cuối; MCP stdio; MCP HTTP JSON/SSE +
  session header + resources/prompts.
- **Số test hiện tại:** `tez-core` 250 + 1 MCP stdio, `tez-auth` 32,
  `tez-cli` 18, `tez-llm` 14, `tez-store` 28, `tez-update` 16; tổng 359.
  Desktop `cargo test` trong `apps/desktop/src-tauri` có 9 test. `cargo test
  --workspace` xanh.
- **Lint/format:** `cargo clippy --all-targets -- -D warnings` và
  `cargo fmt --all -- --check` sạch.
- **Frontend:** `pnpm exec tsc --noEmit` và `pnpm build` (Vite) sạch; preview
  tĩnh với Tauri mock dùng để kiểm tra approval modal, Harness dock và
  Settings → Harness.
- **Trajectory eval:** model có gọi đúng tool? có bỏ qua tool cần thiết? có lộ đáp án?
- **CLI e2e:** `assert_cmd`; connect/chat/tools/context (mở rộng cho memory và
  permission).
- **Desktop smoke:** Playwright/Tauri driver (sau).
- **Eval bộ môn:** 8 case/môn + 20 case chung; promptfoo hoặc harness Rust.
- **Regression:** mỗi prompt/tool change chạy lại; so sánh pass rate.

---

## 15. Lộ trình thực tế

| Phase | Thời lượng | Nội dung | Kết quả |
|---|---|---|---|
| 0. Rust basics | 1–2 tuần | Ownership, Result, trait, async, serde, SQLite | Viết được CLI nhỏ |
| 1. Core + CLI | 2–3 tuần | workspace, provider OpenAI-compatible, `tez chat` | Chat streaming (desktop + CLI) |
| 2. Auth + store | 2 tuần | SQLite, keychain, vault, `tez connect/models` | BYOK hoạt động |
| 3. Agent loop | 2–3 tuần | context builder, tool registry, 6 tool chung | Agent biết gọi tool |
| 4. Subject packs | 2–3 tuần | 4 pack mỏng, RAG, evidence, eval | Dạy được 4 môn cơ bản |
| 5. Tauri | 2–3 tuần | shell, UI chat, tool cards, context inspector | Desktop app |
| 6. Packaging | 1–2 tuần | installer, CI, beta | Người khác cài được |

**Tổng:** ~12–18 tuần part-time (10–15h/tuần); 5h/tuần thì 6–8 tháng. Đây là dự án Rust đầu tiên nên đừng rút ngắn phase 0.

---

## 16. Lộ trình học Rust đi kèm

1. Ownership/borrowing, `Result`/`Option`, pattern matching.
2. Struct/enum/trait; error handling (`thiserror`, `anyhow`).
3. Iterators, collections, modules, Cargo workspace.
4. Async/await, `tokio`, channels.
5. `serde` JSON/TOML; `reqwest`; SSE.
6. `rusqlite` + migrations; `keyring`; `aes-gcm`.
7. Testing (`cargo test`, `wiremock`, `assert_cmd`).
8. Tauri commands/state/capabilities.

**Mẹo:** viết CLI trước, desktop sau. CLI buộc bạn giữ core sạch và dễ test.

---

## 17. Câu hỏi còn mở

1. **CAS cho Toán:** chỉ numeric/step-check trong MVP, hay thêm sidecar Python/SymPy?
2. **Grammar cho Anh:** `harper` đủ tốt, hay dùng model + rubric?
3. **Tool write:** có cho `record_evidence` tự động không, hay cần xác nhận?
4. **Subject pack:** build-in compile-time hay load từ file để giáo viên sửa?
5. **Lesson pack `.tezlesson`:** có trong MVP không?
6. **Tauri updater:** bật ngay hay để sau khi có bản stable?

---

*Tài liệu liên quan: [PLAN.md](PLAN.md) · [DESKTOP-CLI.md](DESKTOP-CLI.md) · [PROVIDER-AUTH.md](PROVIDER-AUTH.md)*
