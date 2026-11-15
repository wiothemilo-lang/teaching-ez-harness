//! Plugin local: một thư mục trong `plugins/<id>/` với `plugin.json`.
//!
//! Plugin là gói cấu hình, không phải code chạy trực tiếp. Nó có thể mang:
//! - `mcpServers`: server MCP stdio,
//! - `skills`: các file/thư mục skill Markdown.
//!
//! Cách này giữ bề mặt tin cậy nhỏ: muốn thêm tool thì dùng MCP, muốn thêm
//! hướng dẫn sư phạm thì dùng skill.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::mcp::McpServerConfig;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
}

impl Plugin {
    /// Đường dẫn tuyệt đối của các skill được khai báo trong manifest.
    ///
    /// Chỉ giữ các đường dẫn thật sự nằm trong thư mục plugin: chặn `..`,
    /// đường dẫn tuyệt đối và symlink trỏ ra ngoài.
    pub fn skill_paths(&self) -> Vec<PathBuf> {
        let Ok(root) = std::fs::canonicalize(&self.dir) else {
            return Vec::new();
        };
        self.manifest
            .skills
            .iter()
            .filter_map(|relative| {
                let path = Path::new(relative);
                let candidate = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    self.dir.join(path)
                };
                let canonical = std::fs::canonicalize(candidate).ok()?;
                canonical.starts_with(&root).then_some(canonical)
            })
            .collect()
    }
}

pub fn plugins_dir(dir: impl AsRef<Path>) -> PathBuf {
    dir.as_ref().join("plugins")
}

/// Nạp mọi plugin hợp lệ trong `plugins/*/plugin.json`.
pub fn load_all(dir: impl AsRef<Path>) -> Result<Vec<Plugin>, PluginError> {
    let root = plugins_dir(dir);
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    let mut plugins = Vec::new();
    for entry in entries {
        if !entry.is_dir() {
            continue;
        }
        let manifest_path = entry.join("plugin.json");
        if !manifest_path.is_file() {
            continue;
        }
        let bytes = match std::fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        plugins.push(Plugin {
            manifest,
            dir: entry,
        });
    }
    Ok(plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_plugin_manifest_and_skill_paths() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("demo");
        std::fs::create_dir_all(plugin_dir.join("skills").join("demo")).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "id": "demo",
                "name": "Demo",
                "version": "0.1.0",
                "description": "test",
                "skills": ["skills/demo"],
                "mcpServers": {
                    "echo": { "command": "python3", "args": ["server.py"] }
                }
            }"#,
        )
        .unwrap();

        let plugins = load_all(dir.path()).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.id, "demo");
        assert_eq!(plugins[0].manifest.mcp_servers.len(), 1);
        assert_eq!(plugins[0].skill_paths()[0], plugin_dir.join("skills/demo"));
    }

    #[test]
    fn skips_invalid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("plugins").join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("plugin.json"), "{ not json").unwrap();

        let plugins = load_all(dir.path()).unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn skill_paths_stay_inside_plugin_dir() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("demo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::create_dir_all(dir.path().join("outside")).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"demo","name":"Demo","skills":["../outside","/etc"]}"#,
        )
        .unwrap();

        let plugins = load_all(dir.path()).unwrap();
        assert!(plugins[0].skill_paths().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn skill_paths_reject_symlink_escape() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("demo");
        std::fs::create_dir_all(plugin_dir.join("skills")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, plugin_dir.join("skills").join("escape")).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"demo","name":"Demo","skills":["skills/escape"]}"#,
        )
        .unwrap();

        let plugins = load_all(dir.path()).unwrap();
        assert!(plugins[0].skill_paths().is_empty());
    }
}
