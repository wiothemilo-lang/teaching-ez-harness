//! Cấu hình harness: `harness.json` trong data dir.
//!
//! Người dùng chỉnh được permission mode, giới hạn agent, compaction, memory và
//! hook shell. File được tạo với giá trị mặc định khi lưu lần đầu.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::compaction::CompactSettings;
use crate::hooks::{HookConfig, HookEvent};
use crate::permission::{PermissionMode, PermissionRule};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_recall: bool,
    #[serde(default = "default_recall_limit")]
    pub recall_limit: usize,
}

fn default_true() -> bool {
    true
}
fn default_recall_limit() -> usize {
    3
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_recall: true,
            recall_limit: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub permission_rules: Vec<PermissionRule>,
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_max_tool_output")]
    pub max_tool_output: usize,
    #[serde(default = "default_subagent_max_steps")]
    pub subagent_max_steps: usize,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default)]
    pub compaction: CompactSettings,
    #[serde(default)]
    pub memory: MemorySettings,
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
}

fn default_max_steps() -> usize {
    12
}
fn default_max_tokens() -> u32 {
    1_024
}
fn default_temperature() -> f32 {
    0.2
}
fn default_top_p() -> f32 {
    1.0
}
fn default_max_tool_output() -> usize {
    8 * 1024
}
fn default_subagent_max_steps() -> usize {
    6
}
fn default_language() -> String {
    "vi".to_string()
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            permission_mode: PermissionMode::Standard,
            permission_rules: Vec::new(),
            max_steps: default_max_steps(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            top_p: default_top_p(),
            max_tool_output: default_max_tool_output(),
            subagent_max_steps: default_subagent_max_steps(),
            system_prompt: String::new(),
            language: default_language(),
            disabled_tools: Vec::new(),
            compaction: CompactSettings::default(),
            memory: MemorySettings::default(),
            hooks: Vec::new(),
        }
    }
}

impl HarnessConfig {
    pub fn sanitized(&self) -> Self {
        let mut config = self.clone();
        config.max_steps = config.max_steps.clamp(1, 100);
        config.max_tokens = config.max_tokens.clamp(128, 32_768);
        config.temperature = config.temperature.clamp(0.0, 2.0);
        config.top_p = config.top_p.clamp(0.0, 1.0);
        config.max_tool_output = config.max_tool_output.clamp(1_024, 65_536);
        config.subagent_max_steps = config.subagent_max_steps.clamp(1, 50);
        if config.language != "en" {
            config.language = "vi".to_string();
        }
        config.system_prompt = config.system_prompt.trim().to_string();
        if config.system_prompt.chars().count() > 8_000 {
            config.system_prompt = config.system_prompt.chars().take(8_000).collect();
        }
        config.disabled_tools.sort();
        config.disabled_tools.dedup();
        config
            .permission_rules
            .retain(|rule| !rule.tool.trim().is_empty());
        for rule in &mut config.permission_rules {
            rule.tool = rule.tool.trim().to_string();
            rule.pattern = rule.pattern.trim().to_string();
        }
        config.permission_rules.truncate(200);
        config.compaction = CompactSettings {
            enabled: config.compaction.enabled,
            ..config.compaction.clone()
        };
        config.memory.recall_limit = config.memory.recall_limit.clamp(0, 10);
        // Hook phải có event hợp lệ và command không rỗng mới được chạy.
        config.hooks.retain_mut(|hook| {
            let Some(event) = HookEvent::parse(&hook.event) else {
                return false;
            };
            let command = hook.command.trim();
            if command.is_empty() {
                return false;
            }
            hook.event = event.as_str().to_string();
            hook.command = command.to_string();
            hook.timeout_secs = hook.timeout_secs.clamp(1, 60);
            true
        });
        config.hooks.truncate(50);
        config
    }
}

pub fn config_path(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join("harness.json")
}

fn field<T: DeserializeOwned>(object: &serde_json::Map<String, Value>, key: &str, default: T) -> T {
    object
        .get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or(default)
}

fn parse_rules(value: Option<&Value>) -> Vec<PermissionRule> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_hooks(value: Option<&Value>) -> Vec<HookConfig> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Nạp config theo từng field: một field hỏng chỉ mất field đó, không kéo
/// toàn bộ cấu hình (đặc biệt là permission mode) về mặc định.
pub fn load(data_dir: impl AsRef<Path>) -> HarnessConfig {
    let path = config_path(data_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return HarnessConfig::default();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return HarnessConfig::default();
    };
    let Some(object) = value.as_object() else {
        return HarnessConfig::default();
    };

    let defaults = HarnessConfig::default();
    let config = HarnessConfig {
        permission_mode: field(object, "permission_mode", defaults.permission_mode),
        permission_rules: parse_rules(object.get("permission_rules")),
        max_steps: field(object, "max_steps", defaults.max_steps),
        max_tokens: field(object, "max_tokens", defaults.max_tokens),
        temperature: field(object, "temperature", defaults.temperature),
        top_p: field(object, "top_p", defaults.top_p),
        max_tool_output: field(object, "max_tool_output", defaults.max_tool_output),
        subagent_max_steps: field(object, "subagent_max_steps", defaults.subagent_max_steps),
        system_prompt: field(object, "system_prompt", defaults.system_prompt),
        language: field(object, "language", defaults.language),
        disabled_tools: field(object, "disabled_tools", defaults.disabled_tools),
        compaction: field(object, "compaction", defaults.compaction),
        memory: field(object, "memory", defaults.memory),
        hooks: parse_hooks(object.get("hooks")),
    };
    config.sanitized()
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut last_error = None;
    for _ in 0..8 {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_file_name(format!(".harness-{}-{suffix}.tmp", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        // create_new: không đi theo symlink/temp file do người khác đặt trước.
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return result;
    }
    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "không tạo được file tạm cho harness config",
        )
    }))
}

pub fn save(data_dir: impl AsRef<Path>, config: &HarnessConfig) -> Result<(), String> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("không tạo được thư mục config: {error}"))?;
    }
    let json = serde_json::to_vec_pretty(&config.sanitized())
        .map_err(|error| format!("không serialize được harness config: {error}"))?;
    write_private_atomic(&path, &json)
        .map_err(|error| format!("không ghi được harness config: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(dir.path());
        assert_eq!(config.permission_mode, PermissionMode::Standard);
        assert!(config.compaction.enabled);
        assert!(config.memory.auto_recall);
    }

    #[test]
    fn roundtrips_and_sanitizes() {
        let dir = tempfile::tempdir().unwrap();
        let config = HarnessConfig {
            permission_mode: PermissionMode::Strict,
            max_steps: 999,
            memory: MemorySettings {
                recall_limit: 99,
                ..MemorySettings::default()
            },
            ..HarnessConfig::default()
        };
        save(dir.path(), &config).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.permission_mode, PermissionMode::Strict);
        assert_eq!(loaded.max_steps, 100);
        assert_eq!(loaded.memory.recall_limit, 10);
    }

    #[test]
    fn bad_field_does_not_reset_security_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            config_path(dir.path()),
            r#"{
                "permission_mode": "trust",
                "max_steps": "không phải số",
                "temperature": 0.5,
                "hooks": [
                    {"event": "stop", "command": "echo ok"},
                    {"event": "không tồn tại", "command": "echo bad"},
                    {"event": "pre_tool_use", "command": "   "}
                ]
            }"#,
        )
        .unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded.permission_mode, PermissionMode::Trust);
        assert_eq!(loaded.max_steps, HarnessConfig::default().max_steps);
        assert_eq!(loaded.temperature, 0.5);
        assert_eq!(loaded.hooks.len(), 1);
        assert_eq!(loaded.hooks[0].event, "stop");
        assert_eq!(loaded.hooks[0].command, "echo ok");
    }

    #[test]
    fn invalid_hooks_are_dropped_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let config = HarnessConfig {
            hooks: vec![
                HookConfig {
                    event: "stop".into(),
                    command: "  echo ok  ".into(),
                    timeout_secs: 0,
                    enabled: true,
                },
                HookConfig {
                    event: "unknown-event".into(),
                    command: "echo bad".into(),
                    timeout_secs: 5,
                    enabled: true,
                },
                HookConfig {
                    event: "pre_tool_use".into(),
                    command: "   ".into(),
                    timeout_secs: 5,
                    enabled: true,
                },
            ],
            ..HarnessConfig::default()
        };
        save(dir.path(), &config).unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded.hooks.len(), 1);
        assert_eq!(loaded.hooks[0].event, "stop");
        assert_eq!(loaded.hooks[0].command, "echo ok");
        assert_eq!(loaded.hooks[0].timeout_secs, 1);
    }

    #[cfg(unix)]
    #[test]
    fn save_is_private_and_atomic() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &HarnessConfig::default()).unwrap();
        let path = config_path(dir.path());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
