//! Subagent: chạy một nhiệm vụ con trong `messages[]` sạch.
//!
//! Subagent dùng cùng workspace, permission và hook với agent cha, nhưng không
//! có tool `task` (một tầng delegation) và không thấy lịch sử hội thoại cha.
//! Chỉ văn bản cuối cùng quay về làm kết quả tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::{LlmProvider, Message, ToolSpec};

use crate::agent::{Agent, AgentConfig, RunContext, StopReason};
use crate::compaction::CompactSettings;
use crate::hooks::HookRegistry;
use crate::permission::{Approver, PermissionPolicy};
use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};

const SUBAGENT_PROMPT: &str = "Bạn là subagent của tez. Hoàn thành đúng nhiệm vụ con được giao, \
không mở rộng phạm vi. Đọc file và chạy lệnh khi cần, nhưng chỉ trong workspace hiện tại. \
Trả về kết quả cuối cùng ngắn gọn, đủ để agent cha tiếp tục mà không cần đọc lại toàn bộ quá trình.";

pub struct SubagentRuntime {
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: ToolRegistry,
    pub policy: Arc<PermissionPolicy>,
    pub approver: Arc<dyn Approver>,
    pub hooks: Arc<HookRegistry>,
    pub max_steps: usize,
    pub compaction: CompactSettings,
}

impl SubagentRuntime {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            tools,
            policy: Arc::new(PermissionPolicy::default()),
            approver: Arc::new(crate::permission::AutoApprover),
            hooks: Arc::new(HookRegistry::new()),
            max_steps: 12,
            compaction: CompactSettings::default(),
        }
    }
}

pub struct TaskTool {
    runtime: Arc<SubagentRuntime>,
}

impl TaskTool {
    pub fn new(runtime: Arc<SubagentRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task".into(),
            description: "Giao một nhiệm vụ con cho agent riêng với context sạch. Dùng khi cần khảo sát nhiều file hoặc một nhánh công việc độc lập; kết quả trả về là văn bản cuối của subagent."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Nhiệm vụ đầy đủ, tự chứa: mục tiêu, file/khu vực liên quan, định dạng kết quả mong muốn"
                    },
                    "description": {
                        "type": "string",
                        "description": "Nhãn ngắn để hiển thị tiến trình"
                    }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
        }
    }

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(180)
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'prompt'".to_string()))?;
        if prompt.trim().is_empty() {
            return Err(ToolError::InvalidArgs("prompt rỗng".to_string()));
        }

        let runtime = &self.runtime;
        let agent = Agent::new(Arc::clone(&runtime.provider), runtime.tools.clone())
            .with_config(AgentConfig {
                max_steps: runtime.max_steps.clamp(1, 40),
                ..AgentConfig::default()
            })
            .with_policy(Arc::clone(&runtime.policy))
            .with_approver(Arc::clone(&runtime.approver))
            .with_hooks(Arc::clone(&runtime.hooks))
            .with_compactor(crate::agent::compactor_for(
                runtime.compaction.clone(),
                None,
            ));

        let messages = vec![
            Message::system(SUBAGENT_PROMPT),
            Message::user(prompt.trim()),
        ];
        match agent
            .run_with(&runtime.model, messages, RunContext::default())
            .await
        {
            Ok(outcome) => {
                let mut text = outcome.text.trim().to_string();
                if text.is_empty() {
                    text = "(subagent không trả về nội dung)".to_string();
                }
                if outcome.stop_reason == StopReason::MaxSteps {
                    text.push_str("\n\n[subagent đã chạm giới hạn bước]");
                }
                Ok(ToolOutput::ok(text))
            }
            Err(error) => Ok(ToolOutput::error(format!("subagent lỗi: {error}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use tez_llm::{ChatRequest, ChatResponse, LlmError, Usage};

    use super::*;

    struct TextProvider;

    #[async_trait]
    impl LlmProvider for TextProvider {
        fn name(&self) -> &str {
            "text"
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
            let last = request
                .messages
                .last()
                .and_then(|message| message.content.clone())
                .unwrap_or_default();
            Ok(ChatResponse::Text {
                content: format!("đã xử lý: {last}"),
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn returns_final_text_only() {
        let runtime = Arc::new(SubagentRuntime::new(
            Arc::new(TextProvider),
            "mock",
            ToolRegistry::new(),
        ));
        let tool = TaskTool::new(runtime);
        let output = tool
            .execute(json!({"prompt": "đếm file trong src"}))
            .await
            .unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("đã xử lý: đếm file trong src"));
    }

    #[tokio::test]
    async fn rejects_empty_prompt() {
        let runtime = Arc::new(SubagentRuntime::new(
            Arc::new(TextProvider),
            "mock",
            ToolRegistry::new(),
        ));
        let tool = TaskTool::new(runtime);
        assert!(tool.execute(json!({"prompt": "  "})).await.is_err());
    }
}
