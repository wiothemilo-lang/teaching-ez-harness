//! Cấu hình harness: `harness.json` trong data dir.
//!
//! Người dùng chỉnh được permission mode, giới hạn agent, compaction, memory và
//! hook shell. File được tạo với giá trị mặc định khi lưu lần đầu.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compaction::CompactSettings;
use crate::hooks::HookConfig;
use crate::permission::PermissionMode;

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
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
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

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            permission_mode: PermissionMode::Standard,
            max_steps: default_max_steps(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
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
        config.compaction = CompactSettings {
            enabled: config.compaction.enabled,
            ..config.compaction.clone()
        };
        config.memory.recall_limit = config.memory.recall_limit.clamp(0, 10);
        config
    }
}

pub fn config_path(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join("harness.json")
}

/// Nạp config; file thiếu hoặc hỏng thì dùng mặc định.
pub fn load(data_dir: impl AsRef<Path>) -> HarnessConfig {
    let path = config_path(data_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return HarnessConfig::default();
    };
    serde_json::from_slice::<HarnessConfig>(&bytes)
        .map(|config| config.sanitized())
        .unwrap_or_default()
}

pub fn save(data_dir: impl AsRef<Path>, config: &HarnessConfig) -> Result<(), String> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("không tạo được thư mục config: {error}"))?;
    }
    let json = serde_json::to_vec_pretty(&config.sanitized())
        .map_err(|error| format!("không serialize được harness config: {error}"))?;
    std::fs::write(&path, json).map_err(|error| format!("không ghi được harness config: {error}"))
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
}
