//! Provider abstraction + OpenAI-compatible adapter.
//!
//! MVP dùng OpenAI-compatible vì phủ được Groq, OpenRouter, Cloudflare,
//! Ollama, LM Studio, vLLM và nhiều gateway nội bộ.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

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
}

/// Gửi một prompt tối thiểu để kiểm tra provider/model/key.
pub async fn ping(provider: &dyn LlmProvider, model: &str) -> Result<String, LlmError> {
    let response = provider
        .chat(ChatRequest {
            model: model.to_string(),
            messages: vec![Message::user("Reply with the single word: OK")],
            tools: Vec::new(),
            temperature: Some(0.0),
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
            api_key: api_key.into(),
            client: reqwest::Client::new(),
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
        if let Some(max_tokens) = req.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }

        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self.client.post(url);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request.json(&body).send().await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(LlmError::Api {
                status: status.as_u16(),
                body: text,
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

        let content = message
            .get("content")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let calls: Vec<ToolCall> = message
            .get("tool_calls")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
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
}
