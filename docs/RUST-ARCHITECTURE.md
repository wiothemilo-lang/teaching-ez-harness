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
  tez-core/                    # domain + agent runtime + context + tools + pedagogy
  tez-llm/                     # provider trait + adapters + streaming + tool calls
  tez-auth/                    # connections, keychain, vault, OAuth
  tez-store/                   # SQLite, migrations, repositories
  tez-server/                  # axum local API + SSE
  tez-cli/                     # binary `tez`
apps/
  desktop/                     # React + Vite frontend
  desktop/src-tauri/           # Tauri shell (Rust), depends on core/server
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
1. Nhận tin nhắn + session + lesson + subject.
2. Build context (có budget token).
3. Gọi provider với messages + tool schemas.
4. Nếu model trả text cuối → stream, lưu, kết thúc.
5. Nếu model gọi tool:
   a. Validate args (JSON Schema).
   b. Kiểm tra quyền theo age mode/lesson.
   c. Thực thi tool (timeout, output cap).
   d. Ghi tool result vào conversation.
   e. Quay lại bước 2 (tối đa N bước).
6. Nếu hết bước/token → trả lời tóm tắt + gợi ý.
```

**Giới hạn mặc định:**

| Giới hạn | Giá trị |
|---|---|
| Số bước agent | 8 |
| Số tool call / bước | 4 |
| Timeout mỗi tool | 10s |
| Output tool tối đa | 8KB (tóm tắt nếu dài) |
| Retry provider | 2 lần (429/5xx) |
| Token context | theo model, mặc định 32k |

### 3.2 Kiểu dữ liệu (ý tưởng)

```rust
pub struct Agent {
    pub provider: std::sync::Arc<dyn LlmProvider>,
    pub tools: ToolRegistry,
    pub context: ContextBuilder,
    pub policy: AgentPolicy,
    pub store: std::sync::Arc<Store>,
}

pub struct AgentRequest {
    pub profile_id: ProfileId,
    pub session_id: SessionId,
    pub lesson_id: Option<LessonId>,
    pub subject: SubjectId,
    pub user_message: String,
    pub model: ModelRef,
    pub age_mode: AgeMode,
}

pub enum AgentEvent {
    TextDelta(String),
    ToolCall { id: String, name: String, args: serde_json::Value },
    ToolResult { id: String, name: String, ok: bool, preview: String },
    StateChanged(SessionState),
    Usage(Usage),
    Done(AgentOutcome),
    Error(AgentError),
}
```

### 3.3 Streaming

- Provider stream → `TextDelta` ngay khi có token.
- Tool call deltas được tích lũy đến khi đủ JSON.
- Server chuyển thành SSE events cho desktop/CLI.
- UI hiển thị "đang gọi tool…" và card kết quả.

### 3.4 Kết thúc

Agent dừng khi:

- Model trả câu trả lời cuối (không còn tool call).
- Chạm `max_steps` hoặc token budget.
- Người dùng hủy (cancellation token).
- Lỗi không phục hồi (provider auth, tool panic).

---

## 4. Context builder — phần quan trọng nhất

### 4.1 Nguồn context

| Nguồn | Nội dung | Ưu tiên |
|---|---|---|
| **System** | Vai trò, môn, lớp, ngôn ngữ, quy tắc sư phạm | Pinned |
| **Subject pack** | Phong cách dạy, thuật ngữ, công thức, rubric | Pinned |
| **Lesson** | Mục tiêu học tập, tài liệu, rubric, độ khó | Pinned |
| **Session state** | Trạng thái (diagnose/practice/...), hint level, turn count | Pinned |
| **RAG** | Top-k chunk liên quan, kèm citation | Cao |
| **Student model** | Mastery, misconception, lịch sử gần đây | Cao |
| **Summary** | Tóm tắt các lượt cũ | Trung bình |
| **Recent turns** | 6–10 lượt gần nhất nguyên văn | Cao |
| **Tool specs** | Schema các tool được phép | Pinned |

### 4.2 Token budget

```rust
pub struct BudgetPolicy {
    pub total: usize,          // theo model
    pub system_pct: u8,        // 10
    pub objectives_pct: u8,    // 10
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

Khi conversation vượt ngưỡng:

1. Giữ nguyên 6 lượt gần nhất.
2. Tóm tắt các lượt cũ bằng model rẻ (hoặc extractive nếu offline).
3. Lưu summary vào `sessions.summary`.
4. Không tóm tắt mục tiêu/trạng thái/tool specs.

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
    fn spec(&self) -> ToolSpec;                 // name, description, JSON schema
    fn permission(&self) -> ToolPermission;
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError>;
}

pub enum ToolPermission {
    ReadOnly,
    WriteStudentData,
    TeacherOnly,
    Dangerous,   // không dùng trong MVP
}
```

### 5.2 Registry & policy

- Tool registry toàn cục; mỗi subject pack khai báo tool được phép.
- `AgentPolicy` lọc thêm theo age mode:
  - `child`: chỉ read-only + write có xác nhận của giáo viên/phụ huynh.
  - `teen`: read-only + write evidence; không tool nguy hiểm.
  - `adult`: như teen + teacher-only nếu là giáo viên.
- Desktop hiện confirmation cho `WriteStudentData`; CLI có `--yes`.
- Tool args validate bằng JSON Schema (`schemars` + `jsonschema`).
- Tool result luôn được coi là **dữ liệu không tin cậy**; wrap trong delimiter; không thực thi chỉ dẫn trong đó.

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

`crates/tez-core/src/tools/workspace.rs` cung cấp bốn tool gắn với một `Workspace`
(root đã canonicalize):

| Tool | Việc |
|---|---|
| `read_file` | đọc text ≤ 256 KB |
| `write_file` | ghi đè toàn bộ nội dung, tạo thư mục cha |
| `list_files` | liệt kê ≤ 300 entry, 1–4 tầng, bỏ `node_modules`/`target`/`.git` |
| `run_command` | `sh -c` (`cmd /C` trên Windows), timeout 1–120 s, cwd trong workspace |

`Workspace::resolve` ghép path tương đối với root, chuẩn hóa `.`/`..`, rồi
canonicalize phần đã tồn tại để chặn symlink trỏ ra ngoài. Đây là giới hạn ở
tầng ứng dụng, không phải sandbox OS. `tools::code_registry(root)` đăng ký cả
bốn tool; `Mode::Code` trong `crates/tez-core/src/modes.rs` chọn prompt system
gắn workspace và registry này.

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
    fn capabilities(&self) -> Capabilities;
    async fn chat(
        &self,
        req: ChatRequest,
        tools: &[ToolSpec],
    ) -> Result<ChatResponse, LlmError>;
    async fn stream(
        &self,
        req: ChatRequest,
        tools: &[ToolSpec],
    ) -> Result<ChatStream, LlmError>;
}
```

- `ChatRequest`: messages, model, temperature, max_tokens, tool choice.
- `ChatResponse`: text hoặc tool calls + usage.
- `ChatStream`: stream events (delta, tool call delta, usage, done).
- Capability: context window, tool calling, vision, structured output, streaming.

Chi tiết auth/BYOK: [PROVIDER-AUTH.md](PROVIDER-AUTH.md).

---

## 8. Storage

Thêm các bảng cho agent:

```sql
sessions (id, lesson_id, profile_id, state, summary, started_at, ended_at)
messages (id, session_id, role, content, model, tokens_in, tokens_out, created_at)
agent_steps (id, session_id, step_no, model, tokens_in, tokens_out,
             latency_ms, finish_reason, created_at)
tool_calls (id, step_id, name, args_json, result_preview, ok, latency_ms, error)
context_snapshots (id, session_id, step_id, sections_json, tokens_json)
evidence (id, session_id, objective_id, type, score, meta_json)
ai_usage (id, profile_id, connection_id, provider_id, model_id, task,
          tokens_in, tokens_out, latency_ms, status, created_at)
```

- `context_snapshots` giúp debug "model đã thấy gì".
- Không lưu secret; tool args có thể chứa dữ liệu học sinh → retention ngắn, cho phép xóa.
- SQLite WAL; một local server ghi.

---

## 9. Local server API

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

- Tauri 2, React + Vite + Tailwind + shadcn.
- Tauri shell khởi động local axum server (in-process) và inject `{ url, token }` cho frontend.
- Tauri commands: `server_info`, `pick_file`, `open_external`, `app_info`.
- Frontend gọi HTTP local; SSE cho chat.
- Màn hình: Onboarding, Chat, Lessons, Tools, Context Inspector, Settings/Connections, Usage.
- Tool call card: tên, args, kết quả, quyền, thời gian.
- Citation panel: tài liệu/trang/snippet.
- Context panel: section + token count + phần bị cắt.
- Security: Tauri capabilities allowlist, CSP, không load remote content.

Chi tiết đóng gói: [DESKTOP-CLI.md](DESKTOP-CLI.md).

---

## 11. CLI `tez`

```bash
tez init
tez connect
tez connect groq --method key
tez connect ollama
tez models
tez chat --subject math --lesson <id>
tez chat --no-tools
tez tools list
tez tools test calculate --args '{"expression":"2+2"}'
tez context inspect --session <id>
tez trace <session-id>
tez lesson create
tez lesson ingest <file>
tez subject list
tez usage
tez doctor
tez serve
```

- `--json` cho automation.
- Exit code: 0 ok, 2 auth, 3 quota, 4 tool error.
- Streaming text + tool status.
- `tez doctor`: server, DB, keychain, provider, tokenizer, disk.

---

## 12. Đóng gói & CI

- `tauri build` cho desktop: deb/rpm/AppImage, dmg, msi/nsis.
- `cargo build --release -p tez-cli`; phát hành binary qua GitHub Releases.
- `cargo dist` (tùy chọn) cho CLI đa nền tảng.
- GitHub Actions matrix: Linux/macOS/Windows; cache cargo; build Tauri cần system deps.
- Code signing: bỏ qua giai đoạn đầu; ghi rõ cách mở installer unsigned.
- Auto-update: Tauri updater + GitHub Releases (khi có bản stable).

---

## 13. Bảo mật

| Rủi ro | Biện pháp |
|---|---|
| Tool nguy hiểm | Không có shell/code runner trong MVP; permission + allowlist |
| Prompt injection qua tài liệu/tool result | Coi là data; delimiter; không thực thi chỉ dẫn; tool allowlist |
| Key rò rỉ | Keychain + AES-GCM; redaction; không trả secret |
| Local server bị gọi trái phép | 127.0.0.1 + bearer token + Origin check |
| Ghi dữ liệu sai | Write tool cần confirmation; audit; undo nếu có thể |
| Lộ đáp án | Policy Socratic + kiểm tra hint level; eval |
| Dữ liệu trẻ em | Local-only; age mode; không telemetry mặc định |
| OAuth consumer | Experimental, cảnh báo ToS; API key là lõi |

---

## 14. Testing & eval

- **Unit:** tool math/chemistry; context budget; vault; policy.
- **Integration:** mock provider + agent loop; tool call → result → final.
- **Trajectory eval:** model có gọi đúng tool? có bỏ qua tool cần thiết? có lộ đáp án?
- **CLI e2e:** `assert_cmd`; connect/chat/tools/context.
- **Desktop smoke:** Playwright/Tauri driver (sau).
- **Eval bộ môn:** 8 case/môn + 20 case chung; promptfoo hoặc harness Rust.
- **Regression:** mỗi prompt/tool change chạy lại; so sánh pass rate.

---

## 15. Lộ trình thực tế

| Phase | Thời lượng | Nội dung | Kết quả |
|---|---|---|---|
| 0. Rust basics | 1–2 tuần | Ownership, Result, trait, async, serde, SQLite | Viết được CLI nhỏ |
| 1. Core + CLI | 2–3 tuần | workspace, provider OpenAI-compatible, `tez chat` | Chat streaming |
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
