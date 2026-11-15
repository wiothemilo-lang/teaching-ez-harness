//! Subagent: chạy một nhiệm vụ con trong `messages[]` sạch.
//!
//! Subagent dùng cùng workspace, permission và hook với agent cha, nhưng không
//! có tool `task` (một tầng delegation) và không thấy lịch sử hội thoại cha.
//! Chỉ văn bản cuối cùng quay về làm kết quả tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::{LlmProvider, Message, ToolSpec};
use tokio::sync::watch;

use crate::agent::{Agent, AgentConfig, RunContext, StopReason};
use crate::compaction::CompactSettings;
use crate::hooks::HookRegistry;
use crate::permission::{Approver, DenyApprover, PermissionPolicy};
use crate::todo::{TodoState, TodoWrite};
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
    /// Nhận cùng tín hiệu Stop với agent cha.
    pub cancel: Option<watch::Receiver<bool>>,
    /// Tool bị người dùng tắt trong harness config.
    pub disabled_tools: Vec<String>,
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
            // Không có kênh duyệt riêng thì từ chối thay vì tự duyệt.
            approver: Arc::new(DenyApprover),
            hooks: Arc::new(HookRegistry::new()),
            max_steps: 12,
            compaction: CompactSettings::default(),
            cancel: None,
            disabled_tools: Vec::new(),
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
        // Subagent không bao giờ nhận `task` (một tầng delegation) và phải
        // tôn trọng danh sách tool bị tắt của harness.
        let mut blocked = runtime.disabled_tools.clone();
        blocked.push("task".to_string());
        let mut tools = runtime.tools.clone().without(&blocked);

        // `todo_write` trong registry cha trỏ vào TodoState của cha; thay bằng
        // state riêng để subagent không sửa kế hoạch của phiên chính.
        let todos = Arc::new(TodoState::new());
        if tools.get("todo_write").is_some() {
            tools.register(TodoWrite::new(Arc::clone(&todos)));
        }

        let mut agent = Agent::new(Arc::clone(&runtime.provider), tools)
            .with_config(AgentConfig {
                max_steps: runtime.max_steps.clamp(1, 40),
                ..AgentConfig::default()
            })
            .with_policy(Arc::clone(&runtime.policy))
            .with_approver(Arc::clone(&runtime.approver))
            .with_hooks(Arc::clone(&runtime.hooks))
            .with_todos(Arc::clone(&todos))
            .with_compactor(crate::agent::compactor_for(
                runtime.compaction.clone(),
                None,
            ));
        if let Some(cancel) = runtime.cancel.clone() {
            agent = agent.with_cancel(cancel);
        }

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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use async_trait::async_trait;
    use tez_llm::{ChatRequest, ChatResponse, FunctionCall, LlmError, ToolCall, Usage};

    use super::*;
    use crate::todo::{TodoItem, TodoStatus};
    use crate::tools::calculator::Calculator;

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

    struct NamedTool(&'static str);

    #[async_trait]
    impl Tool for NamedTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.0.into(),
                description: "test".into(),
                parameters: json!({"type": "object"}),
            }
        }

        async fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::ok("ok"))
        }
    }

    struct RecordingProvider {
        requests: Mutex<Vec<Vec<ToolSpec>>>,
    }

    #[async_trait]
    impl LlmProvider for RecordingProvider {
        fn name(&self) -> &str {
            "recording"
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
            self.requests.lock().unwrap().push(request.tools);
            Ok(ChatResponse::Text {
                content: "xong".into(),
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn strips_task_and_disabled_tools_from_subagent() {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
        });
        let mut tools = ToolRegistry::new();
        tools.register(NamedTool("task"));
        tools.register(NamedTool("write_file"));
        tools.register(Calculator);

        let mut runtime = SubagentRuntime::new(provider.clone(), "mock", tools);
        runtime.disabled_tools = vec!["write_file".into()];
        let tool = TaskTool::new(Arc::new(runtime));
        tool.execute(json!({"prompt": "kiểm tra tool"}))
            .await
            .unwrap();

        let requests = provider.requests.lock().unwrap();
        let names: Vec<&str> = requests[0].iter().map(|spec| spec.name.as_str()).collect();
        assert!(!names.contains(&"task"), "{names:?}");
        assert!(!names.contains(&"write_file"), "{names:?}");
        assert!(names.contains(&"calculate"), "{names:?}");
    }

    struct TodoProvider {
        step: AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for TodoProvider {
        fn name(&self) -> &str {
            "todo"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
            if self.step.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(ChatResponse::ToolCalls {
                    content: None,
                    calls: vec![ToolCall {
                        id: "call_todo".into(),
                        kind: "function".into(),
                        function: FunctionCall {
                            name: "todo_write".into(),
                            arguments: json!({
                                "todos": [{"content": "việc con", "status": "in_progress"}]
                            })
                            .to_string(),
                        },
                    }],
                    usage: Usage::default(),
                });
            }
            Ok(ChatResponse::Text {
                content: "đã cập nhật".into(),
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn subagent_todo_state_is_detached_from_parent() {
        let parent = Arc::new(TodoState::new());
        parent
            .replace(vec![TodoItem::new("việc cha", TodoStatus::InProgress)])
            .unwrap();
        let mut tools = ToolRegistry::new();
        tools.register(TodoWrite::new(Arc::clone(&parent)));

        let runtime = Arc::new(SubagentRuntime::new(
            Arc::new(TodoProvider {
                step: AtomicUsize::new(0),
            }),
            "mock",
            tools,
        ));
        let tool = TaskTool::new(runtime);
        let output = tool
            .execute(json!({"prompt": "cập nhật todo"}))
            .await
            .unwrap();

        assert!(!output.is_error);
        let snapshot = parent.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].content, "việc cha");
    }

    struct SlowProvider {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for SlowProvider {
        fn name(&self) -> &str {
            "slow"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(ChatResponse::Text {
                content: "quá muộn".into(),
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn stop_cancels_running_subagent() {
        let provider = Arc::new(SlowProvider {
            calls: AtomicUsize::new(0),
        });
        let (tx, rx) = watch::channel(false);
        let mut runtime = SubagentRuntime::new(provider.clone(), "mock", ToolRegistry::new());
        runtime.cancel = Some(rx);
        let tool = TaskTool::new(Arc::new(runtime));

        let handle = tokio::spawn(async move { tool.execute(json!({"prompt": "chậm"})).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        tx.send(true).unwrap();

        let output = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("Stop phải ngắt subagent")
            .unwrap()
            .unwrap();
        assert!(
            output.content.contains("Đã dừng theo yêu cầu"),
            "{}",
            output.content
        );
    }
}
