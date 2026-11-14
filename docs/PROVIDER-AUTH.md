# Kết nối AI (BYOK) — thiết kế theo mô hình OpenCode

> **Trạng thái:** Draft v0.1
> **Mục tiêu:** người dùng có thể mang tài khoản AI hoặc API key của chính họ vào harness, chọn model, và dùng cho việc dạy/học — giống cách OpenCode `/connect` + `/models`.

---

## 0. MVP đã triển khai: sổ nhiều provider

Bản desktop/CLI hiện tại dùng `tez_auth::providers` thay cho một `connection.json` duy nhất:

- `providers.json` giữ danh sách provider: `id`, `name`, `provider`, `base_url`, `model`, `encrypted_key`; trường `active` trỏ provider đang dùng.
- `connection.json` vẫn được ghi đồng bộ với provider active để CLI/bản desktop cũ đọc được; lần đầu chạy sẽ migrate file cũ thành một entry.
- Mỗi provider có key ciphertext riêng trong cùng vault AES-256-GCM. Sửa provider mà để trống API key sẽ giữ key cũ; xóa provider active sẽ fallback sang provider đầu tiên còn lại.
- CLI: `tez connect` thêm/cập nhật entry và đặt active; `tez provider list|use|remove` quản lý sổ.
- Desktop: tab **Nhà cung cấp** trong Cài đặt hỗ trợ thêm/sửa/xóa/kiểm tra/chuyển active, với preset Groq, Gemini, OpenRouter, OpenAI, Ollama và custom endpoint.

Phần còn lại của tài liệu mô tả kiến trúc đích (local server, OAuth, keychain); các mục chưa có trong MVP được đánh dấu ở README.

---

## 1. Quyết định cốt lõi

Harness **không bán model** và **không bắt buộc dùng key của mình**. Nó là lớp trung gian:

```
Người dùng ── kết nối ──► Provider (API key / OAuth / local)
                │
                ▼
        Harness (teaching logic)
```

Ba cách kết nối:

| Cách | Độ ổn định | Dùng khi |
|---|---|---|
| **API key** (BYOK) | Cao, chính thức | MVP; đa số provider |
| **OAuth / device login** | Trung bình, tùy provider | Provider có OAuth chính thức (GitHub Copilot, Azure, Vertex) |
| **Local / custom endpoint** | Cao, do người dùng kiểm soát | Ollama, LM Studio, vLLM, gateway nội bộ |

**Lợi ích BYOK với ngân sách 0 đồng:** chi phí inference chuyển về người dùng; bạn chỉ trả hosting/DB (free tier). Đây là cách hợp lý nhất để một học sinh 16 tuổi vận hành sản phẩm.

**Đánh đổi:** bạn phải bảo vệ credential của người dùng rất cẩn thận. Key rò rỉ = mất tiền, mất dữ liệu, mất niềm tin.

---

## 2. OpenCode làm gì (tóm tắt để học theo)

Từ tài liệu OpenCode V2:

- **Provider catalog** lấy từ [models.dev](https://models.dev): danh sách provider, model, capability, context limit, giá.
- **`/connect`** hỗ trợ 4 phương thức:
  - API key
  - OAuth (browser URL, device code, authorization code)
  - Command (chạy lệnh auth của provider, lưu stdout làm key)
  - Environment (đọc biến môi trường, không lưu)
- **Nhiều account/provider**: thêm, đổi active, rename, xóa.
- **Lưu credential** trong SQLite của server local; không gửi ra ngoài.
- **Model picker** `/models`, có model mặc định và override theo task.
- **Local discovery**: Ollama, LM Studio, vLLM.
- **Custom provider**: OpenAI-compatible endpoint + model list thủ công.
- **Provider packages/adapters**: OpenAI, Anthropic, Google, Vertex, Azure, Bedrock, OpenRouter, xAI...

Điểm mấu chốt: OpenCode tách **"kết nối tài khoản"** khỏi **"gọi model"**. Harness của bạn nên làm đúng như vậy.

---

## 3. Thực tế về "đăng nhập tài khoản AI"

Đây là phần dễ gây hiểu nhầm nhất. Có 3 loại "login":

### 3.1. API key (chính thức, nên làm ngay)

Người dùng vào dashboard của provider, tạo key, dán vào harness. Ổn định, đúng điều khoản, dễ làm.

### 3.2. OAuth chính thức (làm được, nhưng chỉ một số provider)

| Provider | OAuth chính thức? | Ghi chú |
|---|---|---|
| **GitHub Copilot** | Có — device flow | Cần quyền Copilot Chat; phù hợp học sinh có GitHub Student Pack |
| **Azure OpenAI** | Có — Microsoft Entra ID | Cần `az login` hoặc service principal |
| **Google Vertex AI** | Có — Application Default Credentials | Cần Google Cloud project, không phải tài khoản Gemini consumer |
| **AWS Bedrock** | Có — AWS credential chain | Profile/SSO/role |
| **OpenAI** | Có "Sign in with ChatGPT" trong OpenCode | Đây là luồng của Codex/ChatGPT, **không phải API OAuth chính thức cho bên thứ ba**; dễ gãy, có rủi ro điều khoản |
| **Anthropic Claude Pro/Max** | Từng có trong OpenCode V1 | Không còn là hướng chính thức; Anthropic yêu cầu 18+ |

### 3.3. "Login tài khoản consumer" (ChatGPT Plus, Claude Pro, Gemini Advanced)

**Không nên làm lõi sản phẩm.**

- Hầu hết consumer subscription **không cấp quyền API cho bên thứ ba**.
- Các luồng OAuth kiểu "sign in with ChatGPT/Claude" thường mô phỏng client chính thức; có thể vi phạm điều khoản, token hết hạn bất thường, bị chặn theo vùng/tài khoản.
- OpenCode làm được vì là **công cụ local**, người dùng tự chịu rủi ro, và đội ngũ duy trì adapter liên tục. Một web app dạy học công khai làm điều này ở quy mô lớn sẽ rất rủi ro.
- Thêm nữa: Anthropic 18+, OpenAI cần phụ huynh đồng ý dưới 18 — không phù hợp để làm mặc định cho học sinh.

**Kết luận:** MVP dùng **API key + local + custom endpoint**. OAuth chính thức (Copilot/Azure/Vertex) để phase sau. "Login ChatGPT/Claude" nếu làm thì là **experimental**, có cảnh báo, không cam kết, không dùng cho lớp học thật.

---

## 4. Kiến trúc

```
┌────────────────────────────────────────────────────────────┐
│ UI: Connect Wizard / Account List / Model Picker           │
└───────────────────────────┬────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────┐
│ API: /api/connections (CRUD, test, refresh)                │
│      /api/models (catalog + custom)                        │
│      /api/chat (stream)                                    │
└───────────────────────────┬────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────┐
│ Auth Method Registry                                       │
│ api_key · oauth_redirect · oauth_device · local · custom   │
└───────────────────────────┬────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────┐
│ Credential Vault (AES-256-GCM, master key, key version)    │
└───────────────────────────┬────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────┐
│ Provider Registry + Adapters                               │
│ OpenAI · Anthropic · Google · Groq · OpenRouter ·          │
│ Cloudflare · Copilot · Ollama · OpenAI-compatible          │
└───────────────────────────┬────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────┐
│ Runtime Gateway: resolve → decrypt → call → stream → log   │
└────────────────────────────────────────────────────────────┘
```

**Nguyên tắc:**

- UI **không bao giờ** nhận lại secret đã lưu; chỉ thấy `••••1234`, trạng thái, ngày dùng cuối.
- Mọi lời gọi model đi qua server; không gọi trực tiếp từ browser.
- Mỗi request gắn `user_id` + `connection_id`; không dùng key của người khác.
- Fallback chỉ trong cùng một user (connection dự phòng của chính họ), không fallback sang key của user khác.

---

## 5. Data model

```sql
-- Danh mục provider (seed từ models.dev hoặc JSON nội bộ)
provider_catalog (
  id            text primary key,       -- 'openai', 'groq', 'ollama', ...
  name          text not null,
  auth_methods  jsonb not null,         -- ['api_key','oauth_device']
  base_url      text,
  docs_url      text,
  models        jsonb not null          -- [{id, name, context, capabilities, cost}]
);

-- Kết nối của từng người dùng
connections (
  id                uuid primary key,
  user_id           uuid not null references profiles(id),
  provider_id       text not null references provider_catalog(id),
  label             text not null,      -- 'Tài khoản cá nhân', 'Lớp 10A'
  auth_type         text not null,      -- api_key | oauth | local | custom
  encrypted_payload text not null,      -- ciphertext (base64)
  iv                text not null,
  auth_tag          text not null,
  key_version       int not null default 1,
  metadata          jsonb not null default '{}', -- base_url, account label, scopes
  status            text not null default 'active', -- active | expired | revoked | error
  expires_at        timestamptz,
  last_used_at      timestamptz,
  created_at        timestamptz not null default now()
);

-- Model mặc định / override theo task
model_preferences (
  user_id       uuid primary key,
  default_model text,                   -- 'groq/llama-3.3-70b'
  task_models   jsonb not null default '{}' -- { "tutor.turn": "...", "ingest": "..." }
);

-- Log sử dụng (không chứa secret)
ai_usage (
  id            uuid primary key,
  user_id       uuid not null,
  connection_id uuid,
  provider_id   text not null,
  model_id      text not null,
  task          text,
  tokens_in     int,
  tokens_out    int,
  latency_ms    int,
  status        text,                   -- ok | rate_limited | auth_error | provider_error
  created_at    timestamptz not null default now()
);
```

**RLS:** `connections` chỉ chủ sở hữu đọc/ghi; giáo viên không thấy key của học sinh. Nếu có "classroom connection", tạo bảng `classroom_connections` với quyền dùng có kiểm soát, không trả secret.

---

## 6. Credential Vault

**Mã hóa:**

- `CREDENTIAL_MASTER_KEY`: 32 byte, base64, đặt trong env của server (không commit).
- Mỗi record: IV ngẫu nhiên 12 byte; AES-256-GCM; lưu ciphertext + iv + tag.
- `key_version` để xoay vòng: giải mã bằng key cũ, mã hóa lại bằng key mới.
- Có thể derive per-user key bằng HKDF(master, user_id) để giảm blast radius.

**Quy tắc:**

- Không log payload, header `Authorization`, hay token.
- Không trả secret về client, kể cả admin.
- Mask khi hiển thị: `sk-...abcd`.
- Xóa connection = xóa bản ghi + ghi audit.
- Backup DB phải mã hóa; master key không nằm cùng backup.
- Nếu chạy local/CLI: dùng OS keychain (libsecret / macOS Keychain / Windows Credential Manager), không lưu plaintext SQLite.

**Ví dụ code (Rust, crate `aes-gcm`):**

```rust
use aes_gcm::{
    aead::{Aead, OsRng, rand_core::RngCore},
    Aes256Gcm, KeyInit, Nonce,
};

pub struct Encrypted {
    pub ciphertext: Vec<u8>, // aes-gcm trả ciphertext kèm tag
    pub iv: [u8; 12],
}

pub fn encrypt(key: &[u8; 32], plain: &[u8]) -> anyhow::Result<Encrypted> {
    let cipher = Aes256Gcm::new(key.into());
    let mut iv = [0u8; 12];
    OsRng.fill_bytes(&mut iv);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&iv), plain)
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    Ok(Encrypted { ciphertext, iv })
}

pub fn decrypt(key: &[u8; 32], enc: &Encrypted) -> anyhow::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(&enc.iv), enc.ciphertext.as_ref())
        .map_err(|e| anyhow::anyhow!("decrypt: {e}"))
}
```

**Lưu ý:** crate `aes-gcm` gắn authentication tag vào cuối ciphertext, nên có thể lưu chung một cột `encrypted_payload`; nếu muốn tách `auth_tag` như schema thì cắt 16 byte cuối. Master key lấy từ OS keychain qua crate `keyring`.

---

## 7. Auth methods

### 7.1 Interface

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum AuthStart {
    Form { fields: Vec<FormField> },
    Redirect { url: String, state: String },
    DeviceCode { user_code: String, verification_url: String, interval: u64 },
}

pub struct Credentials {
    pub secret: String,              // API key hoặc access token
    pub refresh_token: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata: serde_json::Value,
}

#[async_trait::async_trait]
pub trait AuthMethod: Send + Sync {
    fn id(&self) -> AuthMethodId;    // api_key | oauth_redirect | oauth_device | local | custom
    async fn start(&self, ctx: &AuthContext) -> Result<AuthStart, AuthError>;
    async fn complete(&self, ctx: &AuthContext, payload: AuthPayload)
        -> Result<Credentials, AuthError>;
    async fn refresh(&self, creds: &Credentials) -> Result<Credentials, AuthError> {
        Ok(creds.clone())
    }
    async fn revoke(&self, creds: &Credentials) -> Result<(), AuthError> {
        Ok(())
    }
}
```

### 7.2 API key

- Form: label + key + (tùy provider) base URL.
- Test ngay: gọi endpoint nhẹ (`/models` hoặc 1 token) trước khi lưu.
- Nếu test fail: không lưu, báo lỗi rõ (401/403/429).

### 7.3 OAuth redirect (desktop/CLI)

- PKCE + `state` ngẫu nhiên, lưu server-side 10 phút.
- Redirect URI cố định, allowlist.
- Desktop mở browser hệ thống; callback bắt ở local server (127.0.0.1) rồi đóng.
- CLI cũng có thể mở browser + localhost callback; nếu không, dùng device flow.
- Chỉ dùng cho provider có OAuth chính thức.

### 7.4 OAuth device flow (CLI/desktop hoặc web)

- Hiển thị `user_code` + URL.
- Poll token endpoint theo `interval`, có backoff.
- Phù hợp GitHub Copilot, CLI/desktop; không cần callback server.

### 7.5 Local

- Không cần secret.
- Discovery: Ollama `http://127.0.0.1:11434/api/tags`, LM Studio `:1234/v1/models`, vLLM `:8000/v1/models`.
- Vì harness chạy local, `127.0.0.1` chính là máy người dùng — đúng như mong muốn.

### 7.6 Custom OpenAI-compatible

- Nhập `baseURL` + API key (tùy chọn) + model ID.
- Local app cho phép `localhost` (chủ đích), nhưng chặn metadata IP (`169.254.169.254`) và cảnh báo URL lạ.
- Nếu sau này có server/cloud mode, áp dụng allowlist HTTPS + chặn private IP + DNS check.

---

## 8. Runtime flow

```
1. User chọn model (provider/model).
2. Server lấy connection active của user cho provider đó.
3. Nếu hết hạn → refresh token; nếu fail → status=expired, yêu cầu reconnect.
4. Decrypt secret trong memory; không ghi log.
5. Adapter build request (stream, tools, structured output).
6. Stream về client; đếm token/latency.
7. Ghi ai_usage (không ghi prompt/secret).
8. Nếu 401 → đánh dấu expired; nếu 429 → báo quota/fallback cùng user; nếu 5xx → retry có giới hạn.
```

**Fallback policy:**

- Chỉ fallback giữa các connection **của cùng user**.
- Không bao giờ dùng key của user khác, kể cả admin.
- Nếu user chỉ có một connection và nó lỗi → báo rõ, gợi ý kết nối provider khác.

**Model resolution:**

- `provider/model` (ví dụ `groq/llama-3.3-70b`).
- Catalog cung cấp capability: tool, vision, context, structured output.
- Nếu task cần tool mà model không hỗ trợ → chặn từ UI, không gửi request lỗi.

---

## 9. UI/UX

**Connect wizard:**

1. Chọn provider (logo, mô tả, free tier?).
2. Chọn phương thức (API key / OAuth / local / custom).
3. Nhập liệu hoặc làm theo hướng dẫn.
4. Test kết nối → hiện model khả dụng.
5. Đặt tên account, chọn model mặc định.

**Account list:**

- Provider, label, trạng thái (active/expired), lần dùng cuối.
- Hành động: đổi active, rename, test lại, xóa.
- Không hiển thị secret.

**Model picker:**

- Nhóm theo provider; hiện context/capability/giá (nếu có).
- Cảnh báo nếu model không phù hợp task (ví dụ không có tool call).

**Demo key (tùy chọn):**

- Key của bạn, giới hạn 5–10 lượt/ngày/user, chỉ model rẻ.
- Dùng để onboarding người chưa có key.
- Log riêng, có thể tắt.

**Classroom mode:**

- Giáo viên kết nối một account, học sinh dùng qua proxy có quota.
- Học sinh không thấy key.
- Giáo viên xem usage, tắt theo lớp.
- Cần chính sách rõ: ai trả tiền, ai chịu trách nhiệm nội dung.

---

## 10. Bảo mật & tuân thủ

| Rủi ro | Biện pháp |
|---|---|
| Key rò rỉ qua log | Redact; không log body/header; test log review |
| Key rò rỉ qua client | Không trả secret; chỉ server dùng |
| DB bị lộ | Mã hóa envelope; master key tách khỏi DB; rotate |
| SSRF qua custom URL | Allowlist HTTPS; chặn private IP; DNS check |
| OAuth CSRF | `state` + PKCE; expiry ngắn |
| Token refresh race | Lock theo connection; single-flight |
| Dùng key người khác | RLS + kiểm tra `user_id` ở mọi query |
| Người dùng dưới 18 | Consent của phụ huynh/giáo viên; không dùng consumer OAuth trái điều khoản |
| Xóa dữ liệu | Xóa connection + audit; hướng dẫn revoke tại provider |
| Lạm dụng demo key | Quota theo user/IP; captcha; chỉ model rẻ |

**Điều khoản:** mỗi provider có điều khoản riêng về việc dùng credential trong ứng dụng bên thứ ba. API key cá nhân thường được phép nếu người dùng tự nguyện; OAuth consumer thì không chắc. Khi public, ghi rõ trong Terms/Privacy.

---

## 11. Provider matrix cho MVP

| Provider | API key | OAuth | Local | Free tier | Ưu tiên |
|---|---|---|---|---|---|
| **Groq** | ✅ | — | — | ~30 RPM, ~1k req/ngày | MVP |
| **Google Gemini** | ✅ | Vertex (ADC) | — | Flash-Lite ~500 req/ngày | MVP |
| **OpenRouter** | ✅ | — | — | ~50 req/ngày | MVP |
| **OpenAI** | ✅ | ChatGPT (thử nghiệm) | — | Không free | MVP (key) |
| **Anthropic** | ✅ | Claude (không chính thức, 18+) | — | Không free | Sau |
| **Cloudflare Workers AI** | ✅ token | — | — | 10k neurons/ngày | MVP |
| **GitHub Copilot** | — | ✅ device flow | — | Copilot Free/Student | Phase 2 |
| **Azure OpenAI** | ✅ | ✅ Entra ID | — | Tùy tài khoản | Phase 2 |
| **Google Vertex** | ✅ service account | ✅ ADC | — | Tùy project | Phase 2 |
| **Ollama** | — | — | ✅ | Local | MVP |
| **LM Studio / vLLM** | — | — | ✅ | Local | Phase 2 |
| **Custom OpenAI-compatible** | ✅ | — | ✅ | Tùy | MVP |

---

## 12. MVP scope & lộ trình

**MVP (2–3 tuần sau khi có chat cơ bản):**

- [ ] Bảng `provider_catalog` + seed 6 provider.
- [ ] Bảng `connections` + vault AES-256-GCM.
- [ ] API CRUD connection + test key.
- [ ] UI connect wizard (API key + custom + Ollama).
- [ ] Model picker + model mặc định.
- [ ] Runtime resolve connection → stream.
- [ ] Log `ai_usage`, mask secret, không lộ key.
- [ ] Test: key sai, key hết hạn, xóa connection, không fallback chéo user.

**Phase 2:**

- [ ] GitHub Copilot device flow.
- [ ] Azure Entra / Vertex ADC.
- [ ] Classroom connection + quota.
- [ ] Demo key có giới hạn.
- [ ] OAuth refresh + rotation.

**Phase 3 (chỉ khi điều khoản cho phép):**

- [ ] Thử nghiệm "login ChatGPT/Claude" như experimental, có cảnh báo, tách khỏi luồng chính.
- [ ] Không quảng cáo tính năng này cho người dùng dưới 18.

---

## 13. Test plan

- Connect đúng key → status active, model list hiện.
- Connect sai key → 401, không lưu.
- Key bị revoke → request 401 → status expired → UI yêu cầu reconnect.
- 429 → báo quota, gợi ý provider khác; không retry vô hạn.
- Xóa connection → không còn dùng được; audit log có ghi.
- Profile A không đọc/ dùng được connection của profile B (kiểm tra query + API).
- Log không chứa secret/prompt nhạy cảm.
- Custom URL `http://127.0.0.1` được phép ở local app; metadata IP bị chặn.
- OAuth state sai/hết hạn bị từ chối.
- Refresh token fail → không crash, status expired.

---

## 14. Câu hỏi còn mở

1. **Provider nào ưu tiên?** Groq + Gemini + OpenRouter + Ollama là đủ MVP.
2. **Có cần "login tài khoản" thật không, hay API key là đủ?** Khuyến nghị: API key trước; OAuth chính thức sau.
3. **Ai trả tiền inference?** BYOK (người dùng) hay demo key (bạn)?
4. **Có classroom/sync ngay không?** Nên có sau khi có 5 người dùng thật.
5. **Có làm "login ChatGPT/Claude" experimental không?** Desktop local khả thi hơn web, nhưng vẫn là rủi ro điều khoản.
6. **Có lesson pack `.tezlesson` trong MVP không?** Giúp chia sẻ bài offline, nhưng thêm việc.

---

*Tài liệu liên quan: [PLAN.md](PLAN.md) — kế hoạch tổng thể.*
