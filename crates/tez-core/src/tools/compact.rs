//! Tool `compact`: agent chủ động yêu cầu nén lịch sử sau khi hoàn thành một giai đoạn.

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

pub struct Compact;

#[async_trait]
impl Tool for Compact {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "compact".into(),
            description: "Yêu cầu nén lịch sử hội thoại sau khi hoàn thành một giai đoạn, khi bước tiếp theo chỉ cần tóm tắt. Tool này không có tham số."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::ok(
            "Đã ghi nhận yêu cầu nén. Lịch sử sẽ được tóm tắt sau khi xử lý xong batch tool này.",
        ))
    }
}
