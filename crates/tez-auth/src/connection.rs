//! Cấu hình provider dùng chung cho CLI và desktop.
//!
//! `connection.json` chỉ lưu ciphertext của API key; khóa giải mã nằm trong
//! `master.key` cùng thư mục dữ liệu (xem [`crate::Vault`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Vault, VaultError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionFile {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub encrypted_key: String,
}

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error("provider 'custom' cần base_url")]
    MissingBaseUrl,
    #[error("provider '{0}' không tồn tại")]
    ProviderNotFound(String),
    #[error("API key đã lưu không phải UTF-8")]
    InvalidKeyEncoding,
}

/// Tên hiển thị mặc định cho từng loại provider.
pub fn display_name(provider: &str) -> &'static str {
    match provider {
        "groq" => "Groq",
        "openrouter" => "OpenRouter",
        "openai" => "OpenAI",
        "gemini" => "Google Gemini",
        "ollama" => "Ollama (local)",
        "custom" => "Custom OpenAI-compatible",
        _ => "Provider",
    }
}

/// Endpoint mặc định cho các provider OpenAI-compatible.
pub fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "groq" => "https://api.groq.com/openai/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "ollama" => "http://127.0.0.1:11434/v1",
        "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai",
        "openai" => "https://api.openai.com/v1",
        _ => "https://api.openai.com/v1",
    }
}

/// Model mặc định, chọn bản rẻ/nhanh để hợp với ngân sách học sinh.
pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "groq" => "llama-3.3-70b-versatile",
        "openrouter" => "meta-llama/llama-3.3-70b-instruct",
        "ollama" => "qwen3:4b",
        "gemini" => "gemini-2.0-flash",
        "openai" => "gpt-4o-mini",
        _ => "gpt-4o-mini",
    }
}

pub fn path(dir: impl AsRef<Path>) -> PathBuf {
    dir.as_ref().join("connection.json")
}

pub fn load(dir: impl AsRef<Path>) -> Result<Option<ConnectionFile>, ConnectionError> {
    let path = path(dir);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// Lưu connection mới. `base_url`/`model` rỗng sẽ dùng mặc định của provider.
pub fn save(
    dir: impl AsRef<Path>,
    provider: &str,
    base_url: Option<&str>,
    model: Option<&str>,
    api_key: &str,
) -> Result<ConnectionFile, ConnectionError> {
    let dir = dir.as_ref();
    let base_url = match base_url.map(str::trim).filter(|value| !value.is_empty()) {
        Some(url) => url.to_string(),
        None if provider == "custom" => return Err(ConnectionError::MissingBaseUrl),
        None => default_base_url(provider).to_string(),
    };
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_model(provider).to_string());

    let vault = Vault::load_or_create(dir)?;
    let encrypted_key = vault.encrypt(api_key.as_bytes())?;
    let connection = ConnectionFile {
        provider: provider.to_string(),
        base_url,
        model,
        encrypted_key,
    };

    std::fs::write(path(dir), serde_json::to_vec_pretty(&connection)?)?;
    restrict_permissions(&path(dir))?;
    Ok(connection)
}

pub fn decrypt_key(
    dir: impl AsRef<Path>,
    connection: &ConnectionFile,
) -> Result<String, ConnectionError> {
    let vault = Vault::load_or_create(dir)?;
    let bytes = vault.decrypt(&connection.encrypted_key)?;
    String::from_utf8(bytes).map_err(|_| ConnectionError::InvalidKeyEncoding)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), ConnectionError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), ConnectionError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save(dir.path(), "groq", None, None, "sk-test").unwrap();

        assert_eq!(saved.provider, "groq");
        assert_eq!(saved.base_url, default_base_url("groq"));
        assert_eq!(saved.model, default_model("groq"));
        assert!(!saved.encrypted_key.contains("sk-test"));

        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.provider, "groq");
        assert_eq!(decrypt_key(dir.path(), &loaded).unwrap(), "sk-test");
    }

    #[test]
    fn custom_requires_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let error = save(dir.path(), "custom", None, None, "").unwrap_err();
        assert!(matches!(error, ConnectionError::MissingBaseUrl));
    }

    #[test]
    fn blank_model_uses_default() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save(dir.path(), "ollama", None, Some("  "), "").unwrap();
        assert_eq!(saved.model, default_model("ollama"));
    }

    #[cfg(unix)]
    #[test]
    fn connection_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), "ollama", None, None, "").unwrap();
        let mode = std::fs::metadata(path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
