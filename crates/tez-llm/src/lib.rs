//! Provider abstraction + OpenAI-compatible adapter.
//!
//! MVP dùng OpenAI-compatible vì phủ được Groq, OpenRouter, Cloudflare,
//! Ollama, LM Studio, vLLM và nhiều gateway nội bộ.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

/// Timeout tổng cho request không streaming. Streaming dùng idle timeout
/// theo từng lần đọc để không cắt ngang câu trả lời dài.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Thời gian chờ tối đa giữa hai lần đọc của một stream.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Số lần thử lại cho lỗi tạm thời (429/5xx, lỗi kết nối/timeout).
const MAX_RETRIES: usize = 2;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(500);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(8);
/// `Retry-After` do server trả có thể rất lớn; không chờ lâu hơn mức này.
const RETRY_AFTER_MAX: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// URL ảnh đính kèm (data URL hoặc http(s)), theo định dạng vision của
    /// OpenAI-compatible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    /// Message user kèm ảnh. `images` là URL (http(s) hoặc data URL).
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            images,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>, content: Option<String>) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls: Some(calls),
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSpec {
    pub fn to_openai(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub enum ChatResponse {
    Text {
        content: String,
        usage: Usage,
    },
    ToolCalls {
        content: Option<String>,
        calls: Vec<ToolCall>,
        usage: Usage,
    },
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("api error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("invalid response: {0}")]
    Invalid(String),
    #[error("missing API key")]
    MissingKey,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;

    /// Như [`LlmProvider::chat`] nhưng phát dần từng đoạn text qua `tx`.
    ///
    /// Provider không hỗ trợ streaming vẫn dùng được: mặc định gọi `chat`
    /// rồi gửi toàn bộ nội dung một lần.
    async fn chat_stream(
        &self,
        req: ChatRequest,
        tx: Option<UnboundedSender<String>>,
    ) -> Result<ChatResponse, LlmError> {
        let response = self.chat(req).await?;
        if let Some(tx) = tx {
            if let ChatResponse::Text { content, .. } = &response {
                if !content.is_empty() {
                    let _ = tx.send(content.clone());
                }
            }
        }
        Ok(response)
    }
}

/// Gửi một prompt tối thiểu để kiểm tra provider/model/key.
pub async fn ping(provider: &dyn LlmProvider, model: &str) -> Result<String, LlmError> {
    let response = provider
        .chat(ChatRequest {
            model: model.to_string(),
            messages: vec![Message::user("Reply with the single word: OK")],
            tools: Vec::new(),
            temperature: Some(0.0),
            top_p: None,
            max_tokens: Some(8),
        })
        .await?;

    let content = match response {
        ChatResponse::Text { content, .. } => content,
        ChatResponse::ToolCalls { content, .. } => content.unwrap_or_default(),
    };
    let trimmed = content.trim();
    Ok(if trimmed.is_empty() {
        "OK".to_string()
    } else {
        trimmed.to_string()
    })
}

/// Adapter cho mọi endpoint OpenAI-compatible.
pub struct OpenAiCompatible {
    name: String,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    /// Khoảng thời gian tối đa chờ dữ liệu mới của một stream.
    idle_timeout: Duration,
}

impl OpenAiCompatible {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into().trim().to_string(),
            // Chỉ giới hạn thời gian kết nối ở cấp client. Request không
            // streaming tự đặt timeout tổng, còn stream dùng idle timeout
            // để câu trả lời dài không bị cắt ở giây thứ 120.
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            idle_timeout: STREAM_IDLE_TIMEOUT,
        }
    }

    /// Đặt idle timeout cho stream (hữu ích cho test và provider chậm).
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Gửi request không streaming, thử lại lỗi tạm thời với backoff.
    async fn send_with_retry(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let mut attempt = 0usize;
        loop {
            // Giữ query string của base_url (vd Azure `?api-version=...`) ở cuối URL.
            let mut request = self.client.post(self.endpoint());
            if !self.api_key.is_empty() {
                request = request.bearer_auth(&self.api_key);
            }
            match request.timeout(REQUEST_TIMEOUT).json(body).send().await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers());
                    let text = response.text().await.unwrap_or_default();
                    // Giới hạn body để không flood UI/log và không lộ toàn bộ phản hồi.
                    let error = LlmError::Api {
                        status: status.as_u16(),
                        body: body_preview(&text),
                    };
                    if attempt < MAX_RETRIES && retryable(&error) {
                        tokio::time::sleep(retry_delay(attempt, retry_after)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => {
                    let error = LlmError::Http(error);
                    if attempt < MAX_RETRIES && retryable(&error) {
                        tokio::time::sleep(retry_delay(attempt, None)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    /// URL chat completions, giữ nguyên query string của `base_url`.
    fn endpoint(&self) -> String {
        let (base, query) = match self.base_url.split_once('?') {
            Some((base, query)) => (base, Some(query)),
            None => (self.base_url.as_str(), None),
        };
        let base = base.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            match query {
                Some(query) => format!("{base}?{query}"),
                None => base.to_string(),
            }
        } else {
            match query {
                Some(query) => format!("{base}/chat/completions?{query}"),
                None => format!("{base}/chat/completions"),
            }
        }
    }
}

fn message_json(m: &Message) -> Value {
    let mut obj = json!({ "role": m.role });
    if m.images.is_empty() {
        // Không có ảnh: giữ nguyên content dạng string/null như cũ.
        if let Some(content) = &m.content {
            obj["content"] = json!(content);
        }
    } else {
        // Có ảnh: content phải là mảng parts theo chuẩn vision.
        let mut parts: Vec<Value> = Vec::new();
        if let Some(content) = &m.content {
            if !content.is_empty() {
                parts.push(json!({ "type": "text", "text": content }));
            }
        }
        for url in &m.images {
            parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
        }
        obj["content"] = Value::Array(parts);
    }
    if let Some(calls) = &m.tool_calls {
        obj["tool_calls"] = json!(calls);
    }
    if let Some(id) = &m.tool_call_id {
        obj["tool_call_id"] = json!(id);
    }
    obj
}

/// Rút gọn body lỗi để không flood UI/log.
fn body_preview(text: &str) -> String {
    text.chars().take(500).collect()
}

fn request_body(req: &ChatRequest, stream: bool) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages.iter().map(message_json).collect::<Vec<_>>(),
    });
    if stream {
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(req
            .tools
            .iter()
            .map(ToolSpec::to_openai)
            .collect::<Vec<_>>());
    }
    if let Some(temperature) = req.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = req.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(max_tokens) = req.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    body
}

fn retryable(error: &LlmError) -> bool {
    match error {
        LlmError::Api { status, .. } => *status == 429 || (500..=599).contains(status),
        LlmError::Http(error) => error.is_connect() || error.is_timeout(),
        _ => false,
    }
}

fn retry_delay(attempt: usize, retry_after: Option<Duration>) -> Duration {
    retry_after.unwrap_or_else(|| {
        RETRY_BASE_DELAY
            .saturating_mul(1u32 << attempt.min(8))
            .min(RETRY_MAX_DELAY)
    })
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let seconds = value.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds).min(RETRY_AFTER_MAX))
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct StreamState {
    content: String,
    calls: BTreeMap<usize, PartialToolCall>,
    usage: Usage,
    next_index: usize,
    /// Đã thấy `[DONE]` hoặc `finish_reason` chưa.
    completed: bool,
}

enum LineAction {
    Continue,
    Done,
    /// Consumer đã đóng kênh; dừng đọc stream.
    Cancelled,
}

impl StreamState {
    /// Xử lý một dòng SSE đã hoàn chỉnh (không gồm `\n`).
    fn handle_line(
        &mut self,
        line: &[u8],
        tx: &UnboundedSender<String>,
    ) -> Result<LineAction, LlmError> {
        // Chỉ decode khi đã có trọn dòng, nhờ vậy ký tự UTF-8 bị chia
        // giữa hai lần đọc không bị hỏng.
        let line = std::str::from_utf8(line)
            .map_err(|error| LlmError::Invalid(format!("stream line không phải UTF-8: {error}")))?;
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(LineAction::Continue);
        };
        let data = data.trim();
        if data.is_empty() {
            return Ok(LineAction::Continue);
        }
        if data == "[DONE]" {
            self.completed = true;
            return Ok(LineAction::Done);
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Ok(LineAction::Continue);
        };

        // Một số provider báo lỗi giữa stream bằng frame `{"error": {...}}`
        // với HTTP status vẫn là 200.
        if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            return Err(LlmError::Invalid(format!("stream error: {message}")));
        }

        if let Some(raw_usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Usage {
                input_tokens: raw_usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                output_tokens: raw_usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            };
        }

        let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) else {
            return Ok(LineAction::Continue);
        };
        // Không dừng ngay: provider có thể gửi thêm frame usage sau
        // finish_reason. Chỉ ghi nhận là stream đã hoàn tất.
        if choice
            .get("finish_reason")
            .is_some_and(|reason| !reason.is_null())
        {
            self.completed = true;
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(LineAction::Continue);
        };
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                self.content.push_str(text);
                if tx.send(text.to_string()).is_err() {
                    return Ok(LineAction::Cancelled);
                }
            }
        }
        if let Some(items) = delta.get("tool_calls").and_then(Value::as_array) {
            for item in items {
                self.merge_tool_call(item);
            }
        }
        Ok(LineAction::Continue)
    }

    fn merge_tool_call(&mut self, item: &Value) {
        let index = self.resolve_index(item);
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                entry.id = id.to_string();
            }
        }
        if let Some(function) = item.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if !name.is_empty() && entry.name.is_empty() {
                    entry.name = name.to_string();
                }
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                entry.arguments.push_str(arguments);
            }
        }
    }

    /// Provider không gửi `index` (một số gateway cũ): cấp index tuần tự
    /// cho từng call mới thay vì dồn tất cả vào slot 0. Fragment arguments
    /// không có id/name được nối vào call gần nhất.
    fn resolve_index(&mut self, item: &Value) -> usize {
        if let Some(index) = item.get("index").and_then(Value::as_u64) {
            let index = index as usize;
            self.next_index = self.next_index.max(index.saturating_add(1));
            return index;
        }

        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if !id.is_empty() {
            if let Some((&index, _)) = self.calls.iter().find(|(_, call)| call.id == id) {
                return index;
            }
            return self.allocate_index();
        }

        let name = item
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !name.is_empty() || self.calls.is_empty() {
            self.allocate_index()
        } else if let Some(&last) = self.calls.keys().next_back() {
            last
        } else {
            self.allocate_index()
        }
    }

    fn allocate_index(&mut self) -> usize {
        let index = self.next_index;
        self.next_index += 1;
        index
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatible {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let body = request_body(&req, false);
        let response = self.send_with_retry(&body).await?;
        let text = response.text().await?;

        let value: Value =
            serde_json::from_str(&text).map_err(|e| LlmError::Invalid(format!("json: {e}")))?;
        let choice = value
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| LlmError::Invalid("no choices".into()))?;
        let message = choice
            .get("message")
            .ok_or_else(|| LlmError::Invalid("no message".into()))?;

        let content = match message.get("content") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            // Một số gateway trả content dạng mảng parts.
            Some(Value::Array(parts)) => {
                let text: String = parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            Some(other) => return Err(LlmError::Invalid(format!("content không hợp lệ: {other}"))),
        };
        let calls: Vec<ToolCall> = match message.get("tool_calls") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| LlmError::Invalid(format!("tool_calls không hợp lệ: {e}")))?,
        };
        let usage = value
            .get("usage")
            .map(|u| Usage {
                input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                output_tokens: u
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            })
            .unwrap_or_default();

        if calls.is_empty() {
            Ok(ChatResponse::Text {
                content: content.unwrap_or_default(),
                usage,
            })
        } else {
            Ok(ChatResponse::ToolCalls {
                content,
                calls,
                usage,
            })
        }
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
        tx: Option<UnboundedSender<String>>,
    ) -> Result<ChatResponse, LlmError> {
        let Some(tx) = tx else {
            return self.chat(req).await;
        };

        let body = request_body(&req, true);
        let mut attempt = 0usize;
        // Chỉ chờ headers; phần thân stream được đọc với idle timeout từng
        // chunk nên câu trả lời dài không bị cắt ở một mốc tổng cố định.
        // Thử lại chỉ diễn ra trước khi server gửi byte SSE đầu tiên.
        let response = loop {
            let mut request = self.client.post(self.endpoint());
            if !self.api_key.is_empty() {
                request = request.bearer_auth(&self.api_key);
            }
            match tokio::time::timeout(self.idle_timeout, request.json(&body).send()).await {
                Ok(Ok(response)) if response.status().is_success() => break response,
                Ok(Ok(response)) => {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers());
                    let text = tokio::time::timeout(self.idle_timeout, response.text())
                        .await
                        .map_err(|_| {
                            LlmError::Invalid("timeout reading error response".into())
                        })??;
                    let error = LlmError::Api {
                        status: status.as_u16(),
                        body: body_preview(&text),
                    };
                    if attempt < MAX_RETRIES && retryable(&error) {
                        tokio::time::sleep(retry_delay(attempt, retry_after)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
                Ok(Err(error)) => {
                    let error = LlmError::Http(error);
                    if attempt < MAX_RETRIES && retryable(&error) {
                        tokio::time::sleep(retry_delay(attempt, None)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
                Err(_) => {
                    if attempt < MAX_RETRIES {
                        tokio::time::sleep(retry_delay(attempt, None)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(LlmError::Invalid(
                        "stream timeout waiting for response".into(),
                    ));
                }
            }
        };

        // 200 nhưng không phải SSE thường là body JSON lỗi của gateway.
        // Trước đây các body này bị bỏ qua và trả về Ok rỗng/một phần.
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.is_empty() && !content_type.contains("text/event-stream") {
            let text = tokio::time::timeout(self.idle_timeout, response.text())
                .await
                .map_err(|_| LlmError::Invalid("timeout reading non-SSE response".into()))??;
            return Err(LlmError::Invalid(format!(
                "expected text/event-stream, got {content_type}: {}",
                body_preview(&text)
            )));
        }

        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        let mut state = StreamState::default();
        let mut stopped = false;
        let mut cancelled = false;

        'stream: loop {
            let chunk = match tokio::time::timeout(self.idle_timeout, stream.next()).await {
                Ok(Some(chunk)) => chunk?,
                Ok(None) => break,
                // Đã thấy finish_reason thì việc server giữ kết nối thêm
                // không làm stream trở thành lỗi.
                Err(_) if state.completed => break,
                Err(_) => {
                    return Err(LlmError::Invalid(format!(
                        "stream idle timeout after {:?}",
                        self.idle_timeout
                    )))
                }
            };
            buffer.extend_from_slice(&chunk);
            while let Some(position) = buffer.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = buffer.drain(..=position).collect();
                // Bỏ '\n'; '\r' (nếu có) do handle_line xử lý.
                let line = &line[..line.len() - 1];
                match state.handle_line(line, &tx)? {
                    LineAction::Continue => {}
                    LineAction::Done => {
                        stopped = true;
                        break 'stream;
                    }
                    LineAction::Cancelled => {
                        cancelled = true;
                        stopped = true;
                        break 'stream;
                    }
                }
            }
        }

        // Dòng cuối có thể không có '\n' trước khi server đóng kết nối.
        if !stopped && !buffer.is_empty() {
            match state.handle_line(&buffer, &tx)? {
                LineAction::Continue | LineAction::Done => {}
                LineAction::Cancelled => cancelled = true,
            }
        }

        // EOF trước [DONE]/finish_reason là stream bị cắt, không phải thành công.
        if !cancelled && !state.completed {
            return Err(LlmError::Invalid(
                "stream ended without [DONE] or finish_reason".into(),
            ));
        }

        let calls: Vec<ToolCall> = state
            .calls
            .into_iter()
            .map(|(index, call)| ToolCall {
                id: if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                },
                kind: "function".to_string(),
                function: FunctionCall {
                    name: call.name,
                    arguments: call.arguments,
                },
            })
            .collect();

        if calls.is_empty() {
            Ok(ChatResponse::Text {
                content: state.content,
                usage: state.usage,
            })
        } else {
            Ok(ChatResponse::ToolCalls {
                content: if state.content.is_empty() {
                    None
                } else {
                    Some(state.content)
                },
                calls,
                usage: state.usage,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const SSE_HEADERS: &[u8] =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";

    /// Server TCP tối giản: ghi từng chunk riêng biệt để mô phỏng dữ liệu
    /// đến không đúng ranh giới (đặc biệt là ký tự UTF-8 bị chia đôi).
    async fn serve_chunks(chunks: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            for chunk in chunks {
                if socket.write_all(&chunk).await.is_err() {
                    return;
                }
                let _ = socket.flush().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let _ = socket.shutdown().await;
        });
        format!("http://{address}")
    }

    /// Server trả lần lượt từng response, mỗi connection một response.
    async fn serve_responses(responses: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://{address}")
    }

    async fn serve_sse(body: &str) -> String {
        let mut response = SSE_HEADERS.to_vec();
        response.extend_from_slice(body.as_bytes());
        serve_chunks(vec![response]).await
    }

    async fn serve_http(content_type: &str, body: &str) -> String {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}"
        );
        serve_chunks(vec![response.into_bytes()]).await
    }

    /// Ghi phần đầu rồi giữ kết nối im lặng để kiểm tra idle timeout.
    async fn serve_stalled(prefix: Vec<u8>, stall: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket.write_all(&prefix).await;
            let _ = socket.flush().await;
            tokio::time::sleep(stall).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{address}")
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "test-model".to_string(),
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            temperature: None,
            top_p: None,
            max_tokens: None,
        }
    }

    fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> Vec<String> {
        let mut deltas = Vec::new();
        while let Ok(delta) = rx.try_recv() {
            deltas.push(delta);
        }
        deltas
    }

    #[tokio::test]
    async fn streams_text_deltas_and_usage() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Xin \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"chào\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        assert_eq!(drain(&mut rx), vec!["Xin ".to_string(), "chào".to_string()]);
        match response {
            ChatResponse::Text { content, usage } => {
                assert_eq!(content, "Xin chào");
                assert_eq!(usage.input_tokens, 7);
                assert_eq!(usage.output_tokens, 2);
            }
            other => panic!("expected text response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn retries_transient_api_errors() {
        let responses = vec![
            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"ok\"}}]}".to_vec(),
        ];
        let base_url = serve_responses(responses).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");

        let response = provider.chat(request()).await.unwrap();

        match response {
            ChatResponse::Text { content, .. } => assert_eq!(content, "ok"),
            other => panic!("expected text response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn retries_transient_stream_errors_before_first_byte() {
        let mut ok = SSE_HEADERS.to_vec();
        ok.extend_from_slice(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
        );
        let responses = vec![
            b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            ok,
        ];
        let base_url = serve_responses(responses).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        assert_eq!(drain(&mut rx), vec!["ok".to_string()]);
        match response {
            ChatResponse::Text { content, .. } => assert_eq!(content, "ok"),
            other => panic!("expected text response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn preserves_utf8_character_split_across_reads() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"chào \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"bạn\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let bytes = body.as_bytes().to_vec();
        // Cắt ngay giữa hai byte của "à" (C3 A0).
        let split = bytes.iter().position(|&byte| byte == 0xC3).unwrap() + 1;
        let base_url = serve_chunks(vec![
            SSE_HEADERS.to_vec(),
            bytes[..split].to_vec(),
            bytes[split..].to_vec(),
        ])
        .await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        assert_eq!(drain(&mut rx), vec!["chào ".to_string(), "bạn".to_string()]);
        match response {
            ChatResponse::Text { content, .. } => assert_eq!(content, "chào bạn"),
            other => panic!("expected text response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mid_stream_error_frame_is_returned() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "data: {\"error\":{\"message\":\"provider overloaded\",\"type\":\"server_error\"}}\n\n",
        );
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let error = provider.chat_stream(request(), Some(tx)).await.unwrap_err();

        assert!(
            error.to_string().contains("provider overloaded"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn non_sse_200_body_is_an_error() {
        let base_url = serve_http(
            "application/json",
            r#"{"error":{"message":"model not found"}}"#,
        )
        .await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let error = provider.chat_stream(request(), Some(tx)).await.unwrap_err();

        assert!(
            error.to_string().contains("model not found"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn eof_without_done_is_an_error() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"cut\"}}]}\n\n";
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let error = provider.chat_stream(request(), Some(tx)).await.unwrap_err();

        assert!(
            error.to_string().contains("without [DONE]"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn parses_trailing_line_without_newline() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Xin \"}}]}\n\n",
            r#"data: {"choices":[{"delta":{"content":"cuối"},"finish_reason":"stop"}]}"#,
        );
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        assert_eq!(drain(&mut rx), vec!["Xin ".to_string(), "cuối".to_string()]);
        match response {
            ChatResponse::Text { content, .. } => assert_eq!(content, "Xin cuối"),
            other => panic!("expected text response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parallel_tool_calls_without_index_get_distinct_slots() {
        let body = concat!(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"call_a","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}},{"id":"call_b","function":{"name":"list_dir","arguments":"{\"path\":\".\"}"}}]}}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        );
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        match response {
            ChatResponse::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].id, "call_a");
                assert_eq!(calls[0].function.name, "read_file");
                assert_eq!(calls[0].function.arguments, r#"{"path":"a"}"#);
                assert_eq!(calls[1].id, "call_b");
                assert_eq!(calls[1].function.name, "list_dir");
                assert_eq!(calls[1].function.arguments, r#"{"path":"."}"#);
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_idle_timeout_aborts_stalled_stream() {
        let mut prefix = SSE_HEADERS.to_vec();
        prefix.extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n");
        let base_url = serve_stalled(prefix, Duration::from_secs(5)).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key")
            .with_idle_timeout(Duration::from_millis(50));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let error = provider.chat_stream(request(), Some(tx)).await.unwrap_err();

        assert!(
            error.to_string().contains("idle timeout"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn stops_stream_when_receiver_is_dropped() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"second\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(rx);

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        match response {
            ChatResponse::Text { content, .. } => assert_eq!(content, "first"),
            other => panic!("expected text response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn assembles_streamed_tool_calls() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let base_url = serve_sse(body).await;
        let provider = OpenAiCompatible::new("test", &base_url, "key");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let response = provider.chat_stream(request(), Some(tx)).await.unwrap();

        match response {
            ChatResponse::ToolCalls {
                content,
                calls,
                usage,
            } => {
                assert!(content.is_none());
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_a");
                assert_eq!(calls[0].kind, "function");
                assert_eq!(calls[0].function.name, "read_file");
                assert_eq!(calls[0].function.arguments, "{\"path\":\"a\"}");
                assert_eq!(usage.input_tokens, 0);
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
    }

    #[test]
    fn plain_message_keeps_string_content() {
        let value = message_json(&Message::user("Xin chào"));
        assert_eq!(value, json!({ "role": "user", "content": "Xin chào" }));

        let serialized = serde_json::to_value(Message::user("hi")).unwrap();
        assert!(serialized.get("images").is_none());
    }

    #[test]
    fn user_with_images_builds_text_then_image_parts() {
        let message = Message::user_with_images(
            "Mô tả ảnh này",
            vec![
                "https://example.com/a.png".to_string(),
                "data:image/png;base64,AAA".to_string(),
            ],
        );
        let value = message_json(&message);
        assert_eq!(
            value,
            json!({
                "role": "user",
                "content": [
                    { "type": "text", "text": "Mô tả ảnh này" },
                    { "type": "image_url", "image_url": { "url": "https://example.com/a.png" } },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAA" } },
                ]
            })
        );
    }

    #[test]
    fn empty_content_with_images_has_only_image_parts() {
        let message = Message::user_with_images("", vec!["https://example.com/a.png".to_string()]);
        let value = message_json(&message);
        assert_eq!(
            value,
            json!({
                "role": "user",
                "content": [
                    { "type": "image_url", "image_url": { "url": "https://example.com/a.png" } },
                ]
            })
        );
    }

    #[test]
    fn deserializes_legacy_message_without_images() {
        let message: Message =
            serde_json::from_str(r#"{"role":"user","content":"Xin chào","tool_calls":null}"#)
                .unwrap();
        assert_eq!(message.role, "user");
        assert_eq!(message.content.as_deref(), Some("Xin chào"));
        assert!(message.images.is_empty());
    }
}
