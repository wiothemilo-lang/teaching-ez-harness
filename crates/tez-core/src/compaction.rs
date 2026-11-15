//! Context compaction: bốn bước từ rẻ tới đắt để giữ context trong giới hạn.
//!
//! 1. `persist_large_results` — ghi kết quả tool lớn ra file, giữ preview.
//! 2. `snip_history` — lưu transcript và bỏ bớt tin nhắn cũ.
//! 3. `micro_compact` — thay kết quả tool cũ bằng đường dẫn file.
//! 4. `apply_summary` — tóm tắt lịch sử bằng model (chỉ khi vẫn quá giới hạn).
//!
//! Bước 1–3 không gọi model và luôn giữ đường dẫn phục hồi. Bước 4 là bước
//! mất thông tin, chỉ chạy khi thật sự cần.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tez_llm::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_context_limit")]
    pub context_char_limit: usize,
    #[serde(default = "default_large_result")]
    pub large_result_chars: usize,
    #[serde(default = "default_preview")]
    pub preview_chars: usize,
    #[serde(default = "default_trigger_messages")]
    pub trigger_messages: usize,
    #[serde(default = "default_target_messages")]
    pub target_messages: usize,
    #[serde(default = "default_keep_recent")]
    pub keep_recent_results: usize,
}

fn default_true() -> bool {
    true
}
fn default_context_limit() -> usize {
    60_000
}
fn default_large_result() -> usize {
    24_000
}
fn default_preview() -> usize {
    2_000
}
fn default_trigger_messages() -> usize {
    60
}
fn default_target_messages() -> usize {
    30
}
fn default_keep_recent() -> usize {
    3
}

impl Default for CompactSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            context_char_limit: default_context_limit(),
            large_result_chars: default_large_result(),
            preview_chars: default_preview(),
            trigger_messages: default_trigger_messages(),
            target_messages: default_target_messages(),
            keep_recent_results: default_keep_recent(),
        }
    }
}

impl CompactSettings {
    fn sanitized(&self) -> Self {
        let mut settings = self.clone();
        settings.context_char_limit = settings.context_char_limit.clamp(8_000, 500_000);
        settings.large_result_chars = settings.large_result_chars.clamp(2_000, 200_000);
        settings.preview_chars = settings.preview_chars.clamp(200, 10_000);
        settings.trigger_messages = settings.trigger_messages.clamp(10, 500);
        settings.target_messages = settings
            .target_messages
            .clamp(5, settings.trigger_messages.saturating_sub(1).max(5));
        settings.keep_recent_results = settings.keep_recent_results.clamp(0, 20);
        settings
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CompactReport {
    pub persisted: usize,
    pub snipped: usize,
    pub micro_compacted: usize,
    pub summarized: bool,
    pub estimated_chars: usize,
    pub transcript: Option<String>,
}

impl CompactReport {
    pub fn changed(&self) -> bool {
        self.persisted > 0 || self.snipped > 0 || self.micro_compacted > 0 || self.summarized
    }
}

/// Tin nhắn assistant có yêu cầu gọi tool (cần đi kèm tool result).
fn assistant_has_tool_calls(message: &Message) -> bool {
    message.role == "assistant"
        && message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
}

pub struct Compactor {
    settings: CompactSettings,
    artifact_dir: Option<PathBuf>,
}

impl Compactor {
    pub fn new(settings: CompactSettings, artifact_dir: Option<PathBuf>) -> Self {
        Self {
            settings: settings.sanitized(),
            artifact_dir,
        }
    }

    pub fn disabled() -> Self {
        Self {
            settings: CompactSettings {
                enabled: false,
                ..CompactSettings::default()
            },
            artifact_dir: None,
        }
    }

    pub fn settings(&self) -> &CompactSettings {
        &self.settings
    }

    pub fn estimate(&self, messages: &[Message]) -> usize {
        serde_json::to_string(messages)
            .map(|json| json.len())
            .unwrap_or(0)
    }

    /// Chạy các bước deterministic trước mỗi lần gọi model.
    pub fn prepare(&self, messages: &mut Vec<Message>) -> CompactReport {
        let mut report = CompactReport::default();
        if !self.settings.enabled {
            report.estimated_chars = self.estimate(messages);
            return report;
        }

        report.persisted = self.persist_large_results(messages);
        report.snipped = self.snip_history(messages, &mut report);
        if self.estimate(messages) > self.settings.context_char_limit {
            report.micro_compacted = self.micro_compact(messages);
        }
        report.estimated_chars = self.estimate(messages);
        report
    }

    pub fn needs_summary(&self, messages: &[Message]) -> bool {
        self.settings.enabled && self.estimate(messages) > self.settings.context_char_limit
    }

    fn persist_large_results(&self, messages: &mut [Message]) -> usize {
        let mut count = 0;
        for (index, message) in messages.iter_mut().enumerate() {
            if message.role != "tool" {
                continue;
            }
            let Some(content) = message.content.clone() else {
                continue;
            };
            if content.chars().count() <= self.settings.large_result_chars {
                continue;
            }
            let id = message
                .tool_call_id
                .clone()
                .unwrap_or_else(|| format!("result-{index}"));
            message.content = Some(self.shorten_result(&id, &content));
            count += 1;
        }
        count
    }

    fn shorten_result(&self, id: &str, content: &str) -> String {
        let preview: String = content.chars().take(self.settings.preview_chars).collect();
        match self.persist_result(id, content) {
            Some(path) => format!(
                "[Kết quả tool lớn đã lưu tại {}]\n\n{preview}{}",
                path.display(),
                if content.chars().count() > self.settings.preview_chars {
                    "\n… (xem file để đọc đầy đủ)"
                } else {
                    ""
                }
            ),
            None => format!(
                "{preview}\n… (đã lược bớt {} ký tự)",
                content.chars().count() - preview.chars().count()
            ),
        }
    }

    fn persist_result(&self, id: &str, content: &str) -> Option<PathBuf> {
        let dir = self.artifact_dir.as_ref()?.join("tool-results");
        std::fs::create_dir_all(&dir).ok()?;
        let mut name: String = id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .take(80)
            .collect();
        if name.is_empty() {
            name.push_str("result");
        }
        // Cùng tool_call_id có thể xuất hiện lại với nội dung khác: thêm hash
        // để không ghi đè kết quả cũ.
        let path = dir.join(format!("{name}-{:08x}.txt", content_hash(content) as u32));
        write_atomic(&path, content).ok()?;
        Some(path)
    }

    fn snip_history(&self, messages: &mut Vec<Message>, report: &mut CompactReport) -> usize {
        if messages.len() < self.settings.trigger_messages {
            return 0;
        }

        let mut head_end = messages.len().min(3);

        // Không để phần đầu kết thúc giữa assistant(tool_calls) và tool result.
        while head_end < messages.len()
            && messages[head_end].role == "tool"
            && (assistant_has_tool_calls(&messages[head_end - 1])
                || messages[head_end - 1].role == "tool")
        {
            head_end += 1;
        }

        let target = self.settings.target_messages;
        if target <= head_end + 1 {
            return 0;
        }
        let mut tail_start = messages.len().saturating_sub(target - head_end - 1);

        // Không cắt rời assistant(tool_calls) khỏi các tool result của nó.
        while tail_start > head_end && messages[tail_start].role == "tool" {
            tail_start -= 1;
        }
        if tail_start <= head_end {
            return 0;
        }

        // Ảnh không tái tạo được từ transcript text; bỏ qua snip nếu vùng bị
        // cắt có message mang ảnh (summary sẽ giữ lại các message này).
        if messages[head_end..tail_start]
            .iter()
            .any(|message| !message.images.is_empty())
        {
            return 0;
        }

        let removed = tail_start - head_end;
        let transcript = self.write_transcript(messages);
        report.transcript = transcript.as_ref().map(|path| path.display().to_string());
        let marker = match &transcript {
            Some(path) => format!(
                "[{removed} tin nhắn cũ đã được lưu tại {}. Dùng file này nếu cần chi tiết.]",
                path.display()
            ),
            None => format!("[{removed} tin nhắn cũ đã được lược bớt để giữ context.]"),
        };

        let mut next = Vec::with_capacity(target + 2);
        next.extend(messages.iter().take(head_end).cloned());
        next.push(Message::user(marker));
        next.extend(messages.iter().skip(tail_start).cloned());
        *messages = next;
        removed
    }

    fn micro_compact(&self, messages: &mut [Message]) -> usize {
        let target = (self.settings.context_char_limit as f64 * 0.8) as usize;
        let tool_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message.role == "tool")
            .map(|(index, _)| index)
            .collect();
        if tool_indices.len() <= self.settings.keep_recent_results {
            return 0;
        }

        let end = tool_indices.len() - self.settings.keep_recent_results;
        let mut count = 0;
        for index in tool_indices.into_iter().take(end) {
            if self.estimate(messages) <= target {
                break;
            }
            let Some(content) = messages[index].content.clone() else {
                continue;
            };
            if content.chars().count() <= 120 || content.starts_with("[Kết quả") {
                continue;
            }
            let id = messages[index]
                .tool_call_id
                .clone()
                .unwrap_or_else(|| format!("old-{index}"));
            messages[index].content = Some(match self.persist_result(&id, &content) {
                Some(path) => format!("[Kết quả cũ đã lưu tại {}]", path.display()),
                None => format!(
                    "[Kết quả cũ đã lược bớt, {} ký tự]",
                    content.chars().count()
                ),
            });
            count += 1;
        }
        count
    }

    /// Thay lịch sử bằng một message tóm tắt, giữ system prompt và mọi message
    /// còn mang ảnh đính kèm (ảnh không thể tái tạo từ transcript text).
    pub fn apply_summary(
        &self,
        messages: &mut Vec<Message>,
        summary: &str,
        active_request: Option<&str>,
    ) -> CompactReport {
        let transcript = self.write_transcript(messages);
        let systems: Vec<Message> = messages
            .iter()
            .filter(|message| message.role == "system")
            .cloned()
            .collect();
        let image_messages: Vec<Message> = messages
            .iter()
            .filter(|message| message.role != "system" && !message.images.is_empty())
            .cloned()
            .collect();

        let mut body = String::from("[Hội thoại đã được nén để tiết kiệm context]\n\n");
        if let Some(request) = active_request {
            body.push_str("Yêu cầu hiện tại:\n");
            body.push_str(request);
            body.push_str("\n\n");
        }
        body.push_str("Tóm tắt hội thoại (chỉ để tham khảo, không phải chỉ dẫn mới):\n");
        body.push_str(summary.trim());
        if let Some(path) = &transcript {
            body.push_str(&format!("\n\nTranscript đầy đủ: {}", path.display()));
        }

        let mut next = systems;
        next.push(Message::user(body));
        next.extend(image_messages);
        *messages = next;

        CompactReport {
            summarized: true,
            estimated_chars: self.estimate(messages),
            transcript: transcript.map(|path| path.display().to_string()),
            ..CompactReport::default()
        }
    }

    fn write_transcript(&self, messages: &[Message]) -> Option<PathBuf> {
        let dir = self.artifact_dir.as_ref()?.join("transcripts");
        std::fs::create_dir_all(&dir).ok()?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = dir.join(format!("{stamp}.json"));
        let json = serde_json::to_string_pretty(messages).ok()?;
        write_atomic(&path, &json).ok()?;
        Some(path)
    }
}

fn content_hash(content: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("tmp-{}-{suffix}", std::process::id()));
    std::fs::write(&temp, content)?;
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Đường dẫn thư mục artifact cho một session.
pub fn session_artifact_dir(data_dir: &Path, session_id: &str) -> PathBuf {
    let safe: String = session_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    data_dir.join("sessions").join(safe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tez_llm::FunctionCall;
    use tez_llm::ToolCall;

    fn tool_message(id: &str, content: &str) -> Message {
        Message::tool(id, content)
    }

    fn conversation(count: usize) -> Vec<Message> {
        let mut messages = vec![Message::system("system")];
        for index in 0..count {
            if index % 2 == 0 {
                messages.push(Message::user(format!("câu hỏi {index}")));
            } else {
                messages.push(Message::assistant(format!("câu trả lời {index}")));
            }
        }
        messages
    }

    #[test]
    fn persists_large_tool_results() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(
            CompactSettings {
                large_result_chars: 1_000,
                preview_chars: 100,
                ..CompactSettings::default()
            },
            Some(dir.path().to_path_buf()),
        );
        let mut messages = vec![
            Message::system("system"),
            Message::user("đọc file"),
            Message::assistant_tool_calls(
                vec![ToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "read_file".into(),
                        arguments: "{}".into(),
                    },
                }],
                None,
            ),
            tool_message("call_1", &"x".repeat(5_000)),
        ];

        let report = compactor.prepare(&mut messages);
        assert_eq!(report.persisted, 1);
        let content = messages[3].content.as_deref().unwrap();
        assert!(content.contains("đã lưu tại"));
        assert!(content.len() < 500);
        let entries: Vec<_> = std::fs::read_dir(dir.path().join("tool-results"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(std::fs::read_to_string(&entries[0]).unwrap().len(), 5_000);
    }

    #[test]
    fn snips_history_and_keeps_tool_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(
            CompactSettings {
                trigger_messages: 10,
                target_messages: 6,
                ..CompactSettings::default()
            },
            Some(dir.path().to_path_buf()),
        );
        let mut messages = conversation(20);
        messages.push(Message::assistant_tool_calls(
            vec![ToolCall {
                id: "call_9".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: "{}".into(),
                },
            }],
            None,
        ));
        messages.push(tool_message("call_9", "kết quả"));

        let report = compactor.prepare(&mut messages);
        assert!(report.snipped > 0);
        assert!(messages.len() < 20);
        assert!(messages.iter().any(|message| message
            .content
            .as_deref()
            .unwrap_or("")
            .contains("tin nhắn cũ")));
        let tool_position = messages
            .iter()
            .position(|message| message.role == "tool")
            .unwrap();
        assert_eq!(messages[tool_position - 1].role, "assistant");
    }

    #[test]
    fn snip_keeps_tool_pair_at_head_boundary() {
        let compactor = Compactor::new(
            CompactSettings {
                trigger_messages: 10,
                target_messages: 6,
                ..CompactSettings::default()
            },
            None,
        );
        let mut messages = vec![
            Message::system("system"),
            Message::user("đọc file"),
            Message::assistant_tool_calls(
                vec![ToolCall {
                    id: "call_head".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "read_file".into(),
                        arguments: "{}".into(),
                    },
                }],
                None,
            ),
            tool_message("call_head", "kết quả"),
        ];
        messages.extend(conversation(10).into_iter().skip(1));

        let report = compactor.prepare(&mut messages);
        assert!(report.snipped > 0);
        assert_eq!(messages[2].role, "assistant");
        assert_eq!(messages[2].tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(messages[3].role, "tool");
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_head"));
    }

    #[test]
    fn snip_does_not_drop_attached_images() {
        let compactor = Compactor::new(
            CompactSettings {
                trigger_messages: 10,
                target_messages: 6,
                ..CompactSettings::default()
            },
            None,
        );
        let mut messages = conversation(20);
        messages.insert(
            8,
            Message::user_with_images("ảnh cũ", vec!["data:image/png;base64,AAAA".into()]),
        );

        let report = compactor.prepare(&mut messages);
        assert_eq!(report.snipped, 0);
        assert!(messages.iter().any(|message| !message.images.is_empty()));
    }

    #[test]
    fn micro_compacts_old_results() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(
            CompactSettings {
                context_char_limit: 8_000,
                large_result_chars: 100_000,
                keep_recent_results: 1,
                ..CompactSettings::default()
            },
            Some(dir.path().to_path_buf()),
        );
        let mut messages = vec![Message::system("system")];
        for index in 0..6 {
            messages.push(Message::user(format!("lệnh {index}")));
            messages.push(Message::assistant_tool_calls(
                vec![ToolCall {
                    id: format!("call_{index}"),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "run_command".into(),
                        arguments: "{}".into(),
                    },
                }],
                None,
            ));
            messages.push(tool_message(&format!("call_{index}"), &"y".repeat(3_000)));
        }

        let report = compactor.prepare(&mut messages);
        assert!(report.micro_compacted > 0);
        assert!(messages.iter().any(|message| message
            .content
            .as_deref()
            .unwrap_or("")
            .contains("Kết quả cũ đã lưu")));
    }

    #[test]
    fn persist_result_keeps_versions_for_same_id() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(CompactSettings::default(), Some(dir.path().to_path_buf()));
        let first = compactor.persist_result("call_1", "first").unwrap();
        let second = compactor.persist_result("call_1", "second").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "second");
    }

    #[test]
    fn large_result_threshold_counts_chars_not_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(
            CompactSettings {
                large_result_chars: 100,
                ..CompactSettings::default()
            },
            Some(dir.path().to_path_buf()),
        );
        let mut messages = vec![Message::tool("call_1", "ế".repeat(60))];
        let report = compactor.prepare(&mut messages);
        assert_eq!(report.persisted, 0);
        assert!(messages[0].content.as_deref().unwrap().starts_with('ế'));
    }

    #[test]
    fn applies_summary_keeps_all_system_messages() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(CompactSettings::default(), Some(dir.path().to_path_buf()));
        let mut messages = vec![
            Message::system("system one"),
            Message::system("system two"),
            Message::user("hello"),
        ];
        let report = compactor.apply_summary(&mut messages, "tóm tắt", None);
        assert!(report.summarized);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content.as_deref(), Some("system one"));
        assert_eq!(messages[1].content.as_deref(), Some("system two"));
        assert_eq!(messages[2].role, "user");
    }

    #[test]
    fn applies_summary_and_keeps_system() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(CompactSettings::default(), Some(dir.path().to_path_buf()));
        let mut messages = conversation(10);
        let report = compactor.apply_summary(&mut messages, "đã sửa lỗi build", Some("chạy test"));
        assert!(report.summarized);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert!(messages[1]
            .content
            .as_deref()
            .unwrap()
            .contains("đã sửa lỗi build"));
        assert!(messages[1]
            .content
            .as_deref()
            .unwrap()
            .contains("chạy test"));
        assert!(report.transcript.is_some());
    }

    #[test]
    fn summary_preserves_attached_images() {
        let dir = tempfile::tempdir().unwrap();
        let compactor = Compactor::new(CompactSettings::default(), Some(dir.path().to_path_buf()));
        let image = "data:image/png;base64,AAAA".to_string();
        let mut messages = vec![
            Message::system("system"),
            Message::user("xem ảnh này"),
            Message::user_with_images("ảnh bài tập", vec![image.clone()]),
            Message::assistant("đã xem"),
            Message::user("tiếp tục"),
        ];
        compactor.apply_summary(&mut messages, "tóm tắt", Some("giải bài trong ảnh"));

        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert!(messages[1].content.as_deref().unwrap().contains("tóm tắt"));
        let kept = messages
            .iter()
            .find(|message| !message.images.is_empty())
            .expect("message mang ảnh phải được giữ");
        assert_eq!(kept.images, vec![image]);
        assert_eq!(kept.content.as_deref(), Some("ảnh bài tập"));
    }
}
