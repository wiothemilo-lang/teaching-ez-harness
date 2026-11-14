# Kế hoạch xây dựng Teaching EZ Harness

> **v0.5 — Rust + Tauri + CLI `tez`, local-first, BYOK, agent dạy học hoàn chỉnh.**
> Tài liệu chi tiết:
> - [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md) — agent loop, context builder, tools, subject packs.
> - [DESKTOP-CLI.md](DESKTOP-CLI.md) — Tauri desktop, CLI, local server, đóng gói.
> - [PROVIDER-AUTH.md](PROVIDER-AUTH.md) — kết nối AI kiểu OpenCode: API key, OAuth, local endpoint, vault.

---

## 0. Bối cảnh & quyết định

| Yếu tố | Thực tế |
|---|---|
| Người làm | Solo, 16 tuổi, học sinh cấp 3, part-time 10–15 giờ/tuần |
| Ngân sách | 0 đồng |
| Hình thức | **Desktop app (Tauri) + CLI `tez`**, local-first |
| Ngôn ngữ | **Rust** cho core/CLI/desktop backend; React/Vite cho UI |
| Kết nối AI | **BYOK**: API key / OAuth / local endpoint của người dùng |
| Môn học | **Toán, Lý, Hóa, Anh** |
| Đối tượng | Bắt đầu với học sinh cấp 3; mở rộng sau |
| Trọng tâm | **Agent hoàn chỉnh**: model + tools + context + memory + policy |

**Vì sao local-first?**

- Không hosting, không DB cloud, không chi phí vận hành.
- API key không rời máy; phù hợp dữ liệu học sinh.
- Offline với Ollama.
- Desktop cho giáo viên/học sinh; CLI cho power user/automation.
- Sau này thêm sync/classroom bằng server tùy chọn mà không viết lại core.

**Đánh đổi:** học Rust lâu hơn Electron/TypeScript; đóng gói installer phức tạp hơn; chưa có sync đa máy.

---

## 1. Tầm nhìn & nguyên tắc

**Tầm nhìn:** một harness nằm giữa người dạy/người học và AI, biến model của người dùng thành một **gia sư agent** biết dùng công cụ, hiểu context bài học, nhớ tiến độ học sinh, và dạy theo chiến lược sư phạm — không chỉ trả lời chat.

| Nguyên tắc | Cách làm |
|---|---|
| Provider-agnostic | Mọi lời gọi AI qua `tez-llm`; không hard-code SDK trong agent/tool. |
| Agent-first | Model + tools + context + memory + policy; loop do harness kiểm soát. |
| Pedagogy-first | Mục tiêu học tập, chiến lược dạy, đánh giá do mình kiểm soát. |
| Local-first | DB, tài liệu, credential trên máy người dùng. |
| Teacher-in-the-loop | Giáo viên duyệt nội dung, xem trace, đặt quyền tool. |
| Privacy-by-design | Không telemetry mặc định; dữ liệu tối thiểu; keychain. |
| Đo lường được | Trace từng bước agent, tool call, token, latency; eval trước khi đổi prompt/tool. |
| Bắt đầu cực nhỏ | Một môn sâu, bốn môn mỏng; 5–10 người dùng thật. |

**Không làm trong 6 tháng đầu:**

- Không train/fine-tune model.
- Không microservices/Kubernetes.
- Không mobile app.
- Không LMS/LTI, thanh toán, multi-tenant.
- Không tool nguy hiểm (shell/code runner) trong MVP.
- Không hỗ trợ mọi độ tuổi cùng lúc.

---

## 2. Sản phẩm & đối tượng

**Sản phẩm là gì?**

Một app desktop + CLI cài trên máy:

- Giáo viên soạn bài, tải tài liệu, đặt mục tiêu, cấu hình agent.
- Học sinh chat với tutor agent, được gợi ý theo bậc, làm bài luyện.
- Agent dùng tools (tính toán, đồ thị, cân bằng phương trình, tra từ...) và context (bài học, RAG, hồ sơ học sinh).
- Cả hai dùng tài khoản AI của mình (API key/OAuth/local).

**Bắt đầu với ai?**

1. **Học sinh cấp 3 (13–18)** — MVP. Bạn là user thật, có bạn học/giáo viên.
2. **Đại học/người tự học** — cùng engine, age mode khác.
3. **Trẻ nhỏ (dưới 13)** — sau cùng; cần đồng ý phụ huynh, moderation chặt.

**Age mode** là cấu hình: reading level, tone, hint policy, chủ đề, quyền tool, độ dài phiên, retention.

**Chia sẻ:** export/import `.tezlesson` (JSON + tài liệu + prompt/strategy). Sync cloud để sau.

---

## 3. Phạm vi MVP

**Một câu:** người dùng cài app, kết nối AI của mình, tạo bài từ tài liệu; agent dạy có mục tiêu, biết gọi tool và dùng context; CLI làm được các bước chính; mọi thứ lưu local.

**Tính năng bắt buộc:**

1. **Rust workspace + CLI `tez` + local server + Tauri desktop.**
2. **Connect AI:** API key + Ollama + custom OpenAI-compatible; vault; model picker.
3. **Agent loop:** gọi model → tool call → quan sát → lặp → trả lời; streaming events.
4. **Context builder:** system, subject pack, lesson objectives, RAG, student model, session state, recent turns; token budget + compaction.
5. **Tools:** 6 tool chung + tool theo môn; permission theo age mode; JSON Schema validation.
6. **Subject packs:** Toán, Lý, Hóa, Anh (mỏng); Toán sâu trước.
7. **RAG local:** ingest PDF/DOCX/TXT; FTS5 + embedding; citation.
8. **Session state + evidence + mastery đơn giản.**
9. **Trace + context inspector + usage.**
10. **Đóng gói:** installer Linux (và Windows nếu kịp); CLI binary.

**Không cần trong MVP:**

- Chấm rubric tự động, báo cáo phụ huynh.
- Multi-user/RBAC, sync cloud, LMS.
- OAuth consumer (ChatGPT/Claude) — experimental sau.
- CAS đầy đủ (Toán), grammar checker hoàn hảo (Anh).
- Mobile app.

**Tiêu chí nghiệm thu:**

- `tez connect groq` → dán key → `tez chat --subject math` stream được.
- Agent gọi đúng `calculate`, `search_curriculum`, `record_evidence` trong tình huống mẫu.
- `tez context inspect` hiển thị section/token/bị cắt.
- Desktop connect + chat + tool card chạy được.
- 4 môn: mỗi môn ≥ 5 eval case; Toán ≥ 10.
- Key không có trong log/response/DB plaintext; xóa connection có hiệu lực ngay.
- Ingest 1 PDF, chat trả lời có citation.
- 5 bạn học dùng thử 2 tuần; ≥ 3 người quay lại tuần 2.
- Chi phí vận hành: 0 đồng; inference do người dùng trả.

---

## 4. Kiến trúc

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

**Process model:** local server là chủ sở hữu duy nhất của DB, vault, provider calls. Tauri shell khởi động server in-process; CLI tìm server qua `server.json` hoặc tự chạy. Bind `127.0.0.1`, bearer token.

**Workspace:**

```
crates/
  tez-core/     # agent, context, tools, pedagogy, domain
  tez-llm/      # provider trait + OpenAI-compatible + streaming
  tez-auth/     # keychain, vault, connections, OAuth
  tez-store/    # SQLite, migrations, repos
  tez-server/   # axum + SSE
  tez-cli/      # binary `tez`
apps/
  desktop/      # React + Vite
  desktop/src-tauri/
subjects/
  math/ physics/ chemistry/ english/
```

Chi tiết: [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md).

---

## 5. Stack

| Lớp | Chọn | Vì sao |
|---|---|---|
| Ngôn ngữ | **Rust** | Core/CLI/desktop backend một ngôn ngữ; an toàn, nhanh. |
| Desktop | **Tauri 2** | Binary nhỏ, Rust backend, webview hệ thống. |
| UI | **React + Vite + Tailwind + shadcn/ui** | Không cần SSR; nhanh. |
| Local server | **axum + tokio + tower-http** | SSE/streaming tốt. |
| HTTP client | **reqwest + rustls + eventsource-stream** | Provider streaming. |
| DB | **SQLite + rusqlite (bundled) + refinery** | Local, zero cost. |
| Vector | **FTS5 + embedding BLOB + cosine** (sqlite-vec sau) | Đơn giản cho <10k chunk. |
| AI | **Provider trait tự viết; OpenAI-compatible adapter trước** | Phủ Groq/OpenRouter/Cloudflare/Ollama. |
| Credential | **keyring + aes-gcm + argon2** | OS keychain + mã hóa. |
| CLI | **clap + dialoguer/inquire + indicatif** | Nhanh; ratatui sau. |
| Tokenizer | **tiktoken-rs** | Context budget. |
| Logging | **tracing + tracing-subscriber** | Debug có cấu trúc. |
| Test | **cargo test + wiremock + assert_cmd + insta** | Unit/integration/snapshot. |
| CI | **GitHub Actions** | Build matrix 3 OS. |

**Tauri thay vì Electron:** bạn thích Rust; binary nhỏ; backend Rust tự nhiên. Đổi lại: compile lâu, cần system deps, học Tauri.

**Không dùng Next.js:** desktop không cần SSR.

---

## 6. Agent runtime — trái tim của harness

**Công thức:**

```
Agent = Model + Tools + Context + Memory + Policy + Loop
```

### 6.1 Loop

```
1. Nhận tin nhắn + session + lesson + subject.
2. Build context (budget token).
3. Gọi provider với messages + tool schemas.
4. Nếu text cuối → stream, lưu, kết thúc.
5. Nếu tool call:
   - validate args (JSON Schema)
   - kiểm tra quyền theo age mode/lesson
   - thực thi (timeout, output cap)
   - ghi kết quả vào conversation
   - lặp lại (tối đa 8 bước)
6. Hết bước/token → tóm tắt + gợi ý.
```

### 6.2 Context builder

| Nguồn | Nội dung | Ưu tiên |
|---|---|---|
| System | Vai trò, môn, lớp, ngôn ngữ, quy tắc | Pinned |
| Subject pack | Phong cách dạy, thuật ngữ, rubric | Pinned |
| Lesson | Mục tiêu, tài liệu, độ khó | Pinned |
| Session state | Trạng thái, hint level, turn count | Pinned |
| RAG | Top-k chunk + citation | Cao |
| Student model | Mastery, misconception, lịch sử | Cao |
| Summary | Tóm tắt lượt cũ | Trung bình |
| Recent turns | 6–10 lượt gần nhất | Cao |
| Tool specs | Schema tool được phép | Pinned |

- Token budget theo %; pinned luôn giữ; flexible cắt/tóm tắt.
- Compaction: giữ 6 lượt gần nhất, tóm tắt phần cũ.
- `context_snapshots` + `tez context inspect` để debug.

### 6.3 Tools

| Nhóm | Tool MVP |
|---|---|
| Chung | `search_curriculum`, `get_lesson_context`, `get_student_profile`, `record_evidence`, `generate_practice_item`, `escalate_to_teacher` |
| Toán | `calculate`, `plot_function`, `check_step` |
| Lý | `convert_units`, `physics_formula`, `calculate` |
| Hóa | `balance_equation`, `molar_mass`, `stoichiometry`, `periodic_table` |
| Anh | `check_grammar`, `readability`, `vocabulary_lookup` |

- Tool trait + registry + permission (`ReadOnly`, `WriteStudentData`, `TeacherOnly`, `Dangerous`).
- JSON Schema validation; timeout 10s; output cap 8KB.
- Model không có native tool call → prompt-based JSON fallback.
- Tool result là data không tin cậy; không thực thi chỉ dẫn trong đó.

### 6.4 Memory

- Ngắn hạn: messages + summary.
- Dài hạn: mastery/objective, misconception, hint dependency, lịch ôn.
- Không cần BKT/IRT; interface để thay sau.

### 6.5 Policy

- Socratic, scaffolding ladder, formative check.
- Không lộ đáp án trước khi học sinh thử; hint level tăng dần.
- Age mode quyết định quyền tool và tone.
- Teacher có thể duyệt nội dung/trace.

Chi tiết: [RUST-ARCHITECTURE.md](RUST-ARCHITECTURE.md).

---

## 7. Subject packs — 4 môn

Mỗi môn là gói cấu hình + tool, không phải app riêng:

```
subjects/math/
  subject.toml
  prompts/tutor.md
  prompts/planner.md
  rubrics/problem_solving.toml
  evals/*.json
```

| Môn | Trọng tâm | Tool MVP | Độ sâu MVP |
|---|---|---|---|
| **Toán** | Giải thích từng bước, Socratic | calculate, plot, check_step | Sâu nhất; 10 eval |
| **Lý** | Công thức, đơn vị, bài định lượng | convert_units, physics_formula, calculate | 5–8 eval |
| **Hóa** | Cân bằng phương trình, mol | balance_equation, molar_mass, stoichiometry | 5–8 eval |
| **Anh** | Ngữ pháp, viết, từ vựng | check_grammar, readability, vocabulary_lookup | 5–8 eval |

**Chiến lược:** engine + 4 pack mỏng trước; Toán sâu trước; sau đó Hóa/Lý (dùng chung calculator), rồi Anh. Đừng làm 4 môn sâu cùng lúc.

**Agent profiles:** TutorAgent (học sinh), PlannerAgent (giáo viên), GraderAgent/ContentAgent (sau).

---

## 8. Kết nối AI (BYOK)

| Cách | MVP | Ghi chú |
|---|---|---|
| API key | ✅ | Lõi; Groq/Gemini/OpenRouter/OpenAI |
| Ollama/LM Studio/vLLM | ✅ | Local, offline |
| Custom OpenAI-compatible | ✅ | Gateway nội bộ |
| OAuth chính thức (Copilot/Azure/Vertex) | Phase 2 | Device flow/ADC |
| Login ChatGPT/Claude consumer | Experimental | Rủi ro ToS; không làm lõi |

**Lưu trữ:** master key trong OS keychain (`keyring` crate); API key/OAuth token mã hóa AES-256-GCM trong SQLite. Chỉ local server decrypt.

**Runtime:** resolve connection của profile → decrypt trong memory → adapter → stream → log usage. Không fallback sang key người khác.

Chi tiết: [PROVIDER-AUTH.md](PROVIDER-AUTH.md).

### 8.1 Model free

| Provider | Dùng cho | Giới hạn tham khảo |
|---|---|---|
| Groq | Chat demo, nhanh | ~30 RPM, ~1.000 request/ngày |
| Gemini Flash-Lite | Chất lượng, context dài | ~15 RPM, ~500 request/ngày |
| Cloudflare Workers AI | Fallback, embedding | 10.000 neurons/ngày |
| OpenRouter free | Thử model | ~20 RPM, ~50 request/ngày |
| Ollama | Dev/offline | Tùy máy |

### 8.2 Tuổi & điều khoản

- Anthropic 18+; không làm provider chính khi bạn 16.
- OpenAI 13+; dưới 18 cần phụ huynh đồng ý.
- Consumer login: phần lớn không phải OAuth chính thức; experimental, cảnh báo.
- OAuth chính thức: Copilot device flow, Azure Entra, Vertex ADC — phase 2.

---

## 9. Dữ liệu local & RAG

- SQLite: profiles, connections, courses, lessons, objectives, documents, chunks, sessions, messages, agent_steps, tool_calls, context_snapshots, evidence, ai_usage.
- Ingest: copy file → trích text (PDF/DOCX/TXT) → chunk 500–800 token → embed (Ollama/Gemini/Cloudflare) → BLOB.
- Retrieve: FTS5 + cosine; top-k; citation `{document, page, snippet}`.
- Prompt injection: tài liệu là data; delimiter; tool allowlist.
- `.tezlesson`: JSON + tài liệu + prompt/strategy; import/export offline.

---

## 10. An toàn & quyền riêng tư

| Chủ đề | Biện pháp |
|---|---|
| Dữ liệu | Local-only; không telemetry mặc định; xóa DB = xóa hết. |
| Tên học sinh | Không bắt buộc tên thật; nickname/mã. |
| Credential | Keychain + AES-GCM; redaction; mask. |
| Local server | 127.0.0.1 + bearer token + Origin check; token file `0600`. |
| Tool | Permission + allowlist; không shell/code runner MVP. |
| Prompt injection | Tài liệu/tool result là data; không thực thi chỉ dẫn. |
| Nội dung | Moderation; chặn chủ đề ngoài bài; escalation. |
| Trẻ dưới 13 | Chưa hỗ trợ; sau này cần đồng ý phụ huynh. |
| OAuth | Chỉ provider chính thức; consumer experimental. |
| Update | GitHub Releases + checksum; signing sau. |

---

## 11. Chi phí 0 đồng

- Không hosting/server; GitHub Releases + cargo/npm.
- SQLite local; Ollama offline.
- Inference do người dùng trả (BYOK).
- CI free cho repo public.
- Code signing bỏ qua giai đoạn đầu.
- Demo key tùy chọn, giới hạn/ngày.
- Chi phí lớn nhất: thời gian học Rust + đóng gói + hỗ trợ cài đặt.

---

## 12. Lộ trình

| Phase | Thời lượng | Nội dung | Kết quả |
|---|---|---|---|
| 0. Rust basics | 1–2 tuần | Ownership, Result, trait, async, serde | CLI nhỏ |
| 1. Core + CLI | 2–3 tuần | workspace, provider OpenAI-compatible, `tez chat` | Chat streaming |
| 2. Auth + store | 2 tuần | SQLite, keychain, vault, `tez connect/models` | BYOK |
| 3. Agent loop | 2–3 tuần | context builder, tool registry, 6 tool chung | Agent gọi tool |
| 4. Subject packs | 2–3 tuần | 4 pack mỏng, RAG, evidence, eval | 4 môn cơ bản |
| 5. Tauri | 2–3 tuần | shell, UI chat, tool cards, context inspector | Desktop app |
| 6. Packaging | 1–2 tuần | installer, CI, beta | Người khác cài được |

**Tổng:** 12–18 tuần part-time (10–15h/tuần); 5h/tuần thì 6–8 tháng. Đây là dự án Rust đầu tiên nên đừng rút ngắn phase 0.

**Sau MVP:**

- OAuth chính thức (Copilot/Azure/Vertex).
- Assessment/rubric + teacher approval.
- Age mode thứ hai.
- Lesson pack export/import.
- Sync/classroom tùy chọn.
- Open source, blog, demo.

---

## 13. Eval

**Bộ test:**

- 5 case/môn: đúng kiến thức, hiểu sai, đòi đáp án, ngoài phạm vi, an toàn.
- Toán thêm 5 case tool (calculate/plot/check_step).
- 10 case agent trajectory: có gọi đúng tool? có bỏ qua tool? có lộ đáp án?
- 5 case prompt injection.

**Cách chạy:** promptfoo hoặc harness Rust; judge khác provider; giáo viên chấm mẫu; lưu theo prompt/tool version.

**Ngưỡng:** ≥ 90% factual; ≥ 80% đồng thuận giáo viên; ≥ 85% tool selection đúng; 0 lỗi an toàn.

---

## 14. Rủi ro & giảm thiểu

| Rủi ro | Mức | Giảm thiểu |
|---|---|---|
| Rust learning curve | Cao | Phase 0; CLI trước; project nhỏ |
| Compile time/async lifetimes | Trung bình | Workspace nhỏ; tránh abstraction sớm |
| Tauri system deps | Trung bình | CI matrix; test installer sớm |
| CAS Toán không đủ | Trung bình | Numeric/step-check MVP; SymPy sidecar sau |
| Grammar Anh không đủ | Trung bình | Harper/model + rubric; không hứa quá |
| 4 môn quá tải | Cao | Engine + pack mỏng; Toán sâu trước |
| Tool hallucination | Cao | JSON Schema; validate; eval; permission |
| Context overflow | Trung bình | Budget + compaction + inspector |
| Key rò rỉ | Cao | Keychain + AES-GCM; redaction; test |
| OAuth consumer gãy/ToS | Cao | API key lõi; experimental |
| SQLite hỏng/conflict | Trung bình | WAL; một server; backup |
| Installer unsigned | Trung bình | Hướng dẫn mở; signing sau |
| Scope creep | Cao | Non-goals; phase gate |

---

## 15. KPI

- **Cài đặt:** ≥ 80% người thử cài được trong 10 phút (Linux trước).
- **Kết nối:** ≥ 80% kết nối provider đầu tiên trong 5 phút.
- **Agent:** ≥ 85% tool selection đúng trên bộ eval; ≥ 90% câu trả lời có citation khi dùng RAG.
- **Sử dụng:** 5 người dùng thật; 3 người quay lại tuần 2.
- **Chất lượng:** 4 môn có eval; 0 lỗi an toàn nghiêm trọng.
- **Kỹ thuật:** `tez chat` p95 first-token < 3s (provider nhanh); offline Ollama chạy; 0 key trong log.
- **Cá nhân:** giải thích agent loop/context/tools trong 5 phút; repo public + demo.

---

## 16. Việc cần làm ngay

- [ ] Cài Rust (`rustup`), Node/pnpm, Tauri system deps.
- [ ] Tạo workspace Cargo + `tez-cli` + `tez-core`.
- [ ] CLI `tez chat` streaming với OpenAI-compatible provider (dev key).
- [ ] SQLite + keychain + `tez connect`.
- [ ] Agent loop + context builder + 3 tool chung.
- [ ] Toán pack: `calculate`, `plot_function`, `check_step`.
- [ ] `tez context inspect` + trace.
- [ ] 5 eval case Toán; 1 giáo viên góp ý.

---

## 17. Quyết định đã chốt & còn mở

**Đã chốt:**

- Hình thức: **Tauri desktop + CLI `tez`**, local-first.
- Ngôn ngữ: **Rust** core/CLI/desktop backend; React/Vite UI.
- Agent: model + tools + context + memory + policy + loop.
- Môn: **Toán, Lý, Hóa, Anh**; Toán sâu trước.
- Kết nối: **BYOK**; API key + local + custom; OAuth chính thức phase 2.
- Credential: OS keychain + AES-256-GCM.
- Không hosting, không cloud DB, không multi-tenant MVP.

**Còn mở:**

1. **CAS Toán:** numeric/step-check hay SymPy sidecar?
2. **Grammar Anh:** `harper` hay model-based?
3. **Tool write:** `record_evidence` tự động hay cần xác nhận?
4. **Subject pack:** compile-time hay load từ file?
5. **Lesson pack `.tezlesson`:** có trong MVP không?
6. **Tauri updater:** bật ngay hay sau stable?
7. **Open source:** MIT ngay từ đầu?

---

*Cập nhật lần sau: sau khi cài toolchain và dựng workspace Rust.*
