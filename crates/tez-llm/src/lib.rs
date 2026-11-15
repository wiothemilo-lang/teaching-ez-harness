//! Provider abstraction + OpenAI-compatible adapter.
//!
//! MVP dùng OpenAI-compatible vì phủ được Groq, OpenRouter, Cloudflare,
//! Ollama, LM Studio, vLLM và nhiều gateway nội bộ.

use std::collections::BTreeMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>, content: Option<String>) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls: Some(calls),
            tool_call_id: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
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
            // Không để một provider treo làm kẹt cả agent loop.
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
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
    if let Some(content) = &m.content {
        obj["content"] = json!(content);
    }
    if let Some(calls) = &m.tool_calls {
        obj["tool_calls"] = json!(calls);
    }
    if let Some(id) = &m.tool_call_id {
        obj["tool_call_id"] = json!(id);
    }
    obj
}

#[async_trait]
impl LlmProvider for OpenAiCompatible {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let mut body = json!({
            "model": req.model,
            "messages": req.messages.iter().map(message_json).collect::<Vec<_>>(),
        });

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

        // Giữ query string của base_url (vd Azure `?api-version=...`) ở cuối URL.
        let url = self.endpoint();
        let mut request = self.client.post(url);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request.json(&body).send().await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            // Giới hạn body để không flood UI/log và không lộ toàn bộ phản hồi.
            let body: String = text.chars().take(500).collect();
            return Err(LlmError::Api {
                status: status.as_u16(),
                body,
            });
        }

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

        let mut body = json!({
            "model": req.model,
            "messages": req.messages.iter().map(message_json).collect::<Vec<_>>(),
            "stream": true,
            "stream_options": { "include_usage": true },
        });
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

        let mut request = self.client.post(self.endpoint());
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request.json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await?;
            let body: String = text.chars().take(500).collect();
            return Err(LlmError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut calls: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
        let mut usage = Usage::default();

        'stream: while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            loop {
                let Some(position) = buffer.find('\n') else {
                    break;
                };
                let line: String = buffer.drain(..=position).collect();
                let line = line.trim_end_matches(['\r', '\n']);
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    break 'stream;
                }
                let Ok(value) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(raw_usage) = value.get("usage").filter(|value| !value.is_null()) {
                    usage = Usage {
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
                    continue;
                };
                let Some(delta) = choice.get("delta") else {
                    continue;
                };
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        content.push_str(text);
                        let _ = tx.send(text.to_string());
                    }
                }
                if let Some(items) = delta.get("tool_calls").and_then(Value::as_array) {
                    for item in items {
                        let index = item.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let entry = calls.entry(index).or_default();
                        if let Some(id) = item.get("id").and_then(Value::as_str) {
                            if !id.is_empty() {
                                entry.0 = id.to_string();
                            }
                        }
                        if let Some(function) = item.get("function") {
                            if let Some(name) = function.get("name").and_then(Value::as_str) {
                                if !name.is_empty() && entry.1.is_empty() {
                                    entry.1 = name.to_string();
                                }
                            }
                            if let Some(arguments) =
                                function.get("arguments").and_then(Value::as_str)
                            {
                                entry.2.push_str(arguments);
                            }
                        }
                    }
                }
            }
        }

        let calls: Vec<ToolCall> = calls
            .into_iter()
            .map(|(index, (id, name, arguments))| ToolCall {
                id: if id.is_empty() {
                    format!("call_{index}")
                } else {
                    id
                },
                kind: "function".to_string(),
                function: FunctionCall { name, arguments },
            })
            .collect();

        if calls.is_empty() {
            Ok(ChatResponse::Text { content, usage })
        } else {
            Ok(ChatResponse::ToolCalls {
                content: if content.is_empty() {
                    None
                } else {
                    Some(content)
                },
                calls,
                usage,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve_sse(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
            );
            let _ = socket.write_all(response.as_bytes()).await;
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

        let mut deltas = Vec::new();
        while let Ok(delta) = rx.try_recv() {
            deltas.push(delta);
        }
        assert_eq!(deltas, vec!["Xin ".to_string(), "chào".to_string()]);
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
}
