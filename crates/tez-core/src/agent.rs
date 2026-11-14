use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tez_llm::{ChatRequest, ChatResponse, LlmError, LlmProvider, Message, Usage};

use crate::tools::{ToolError, ToolRegistry};

/// Giới hạn policy cho một tool call.
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TOOL_OUTPUT: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct AgentStep {
    pub tool: String,
    pub args: Value,
    pub output: String,
    pub ok: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub text: String,
    pub steps: Vec<AgentStep>,
    pub usage: Usage,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("llm error: {0}")]
    Llm(#[from] LlmError),
    #[error("agent reached max steps ({0})")]
    MaxSteps(usize),
    #[error("tool error: {0}")]
    Tool(#[from] ToolError),
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    max_steps: usize,
}

impl Agent {
    pub fn new(provider: Arc<dyn LlmProvider>, tools: ToolRegistry) -> Self {
        Self {
            provider,
            tools,
            max_steps: 8,
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub async fn run(
        &self,
        model: &str,
        messages: Vec<Message>,
    ) -> Result<AgentOutcome, AgentError> {
        let mut messages = messages;
        let mut steps = Vec::new();
        let mut total_usage = Usage::default();

        for _ in 0..self.max_steps {
            let request = ChatRequest {
                model: model.to_string(),
                messages: messages.clone(),
                tools: self.tools.specs(),
                temperature: Some(0.2),
                max_tokens: Some(1024),
            };

            match self.provider.chat(request).await? {
                ChatResponse::Text { content, usage } => {
                    total_usage.input_tokens += usage.input_tokens;
                    total_usage.output_tokens += usage.output_tokens;
                    return Ok(AgentOutcome {
                        text: content,
                        steps,
                        usage: total_usage,
                    });
                }
                ChatResponse::ToolCalls {
                    content,
                    calls,
                    usage,
                } => {
                    total_usage.input_tokens += usage.input_tokens;
                    total_usage.output_tokens += usage.output_tokens;
                    messages.push(Message::assistant_tool_calls(calls.clone(), content));

                    for call in calls {
                        let args: Value =
                            serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);

                        let started = Instant::now();
                        let result = tokio::time::timeout(
                            TOOL_TIMEOUT,
                            self.tools.execute(&call.function.name, args.clone()),
                        )
                        .await;
                        let (output, ok) = match result {
                            Ok(Ok(result)) => (cap_output(result.content), !result.is_error),
                            Ok(Err(error)) => (format!("tool error: {error}"), false),
                            Err(_) => (
                                format!("tool timeout sau {}s", TOOL_TIMEOUT.as_secs()),
                                false,
                            ),
                        };
                        let duration_ms = started.elapsed().as_millis() as u64;

                        steps.push(AgentStep {
                            tool: call.function.name.clone(),
                            args,
                            output: output.clone(),
                            ok,
                            duration_ms,
                        });

                        messages.push(Message::tool(call.id.clone(), output));
                    }
                }
            }
        }

        Err(AgentError::MaxSteps(self.max_steps))
    }
}

/// Cắt output tool theo giới hạn 8KB, không cắt giữa ký tự UTF-8.
fn cap_output(mut content: String) -> String {
    if content.len() <= MAX_TOOL_OUTPUT {
        return content;
    }
    let mut end = MAX_TOOL_OUTPUT;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    content.push_str("\n… (đã cắt bớt vì output quá 8KB)");
    content
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use tez_llm::{FunctionCall, ToolCall, Usage};

    use super::*;
    use crate::tools::calculator::Calculator;

    struct MockProvider {
        step: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
            let mut step = self.step.lock().unwrap();
            if *step == 0 {
                *step += 1;
                Ok(ChatResponse::ToolCalls {
                    content: None,
                    calls: vec![ToolCall {
                        id: "call_1".into(),
                        kind: "function".into(),
                        function: FunctionCall {
                            name: "calculate".into(),
                            arguments: "{\"expression\":\"2+2\"}".into(),
                        },
                    }],
                    usage: Usage::default(),
                })
            } else {
                Ok(ChatResponse::Text {
                    content: "Kết quả là 4.".into(),
                    usage: Usage::default(),
                })
            }
        }
    }

    #[tokio::test]
    async fn runs_tool_then_finishes() {
        let mut tools = ToolRegistry::new();
        tools.register(Calculator);

        let agent = Agent::new(
            Arc::new(MockProvider {
                step: Mutex::new(0),
            }),
            tools,
        );

        let outcome = agent
            .run("mock", vec![Message::user("2+2 bằng mấy?")])
            .await
            .unwrap();

        assert_eq!(outcome.text, "Kết quả là 4.");
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].tool, "calculate");
        assert_eq!(outcome.steps[0].output, "4");
        assert!(outcome.steps[0].ok);
        assert_eq!(outcome.usage.input_tokens, 0);
    }

    #[test]
    fn caps_large_tool_output() {
        let output = cap_output("é".repeat(5000));
        assert!(output.len() <= MAX_TOOL_OUTPUT + 64);
        assert!(output.contains("cắt bớt"));
    }
}
