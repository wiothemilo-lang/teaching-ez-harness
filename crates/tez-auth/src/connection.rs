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
    #[error(
        "provider '{0}' không hỗ trợ (chọn: groq, openrouter, openai, gemini, ollama, custom)"
    )]
    ProviderNotFound(String),
    #[error("API key đã lưu không phải UTF-8")]
    InvalidKeyEncoding,
    #[error(
        "API key rỗng cho '{0}'. Nhập key thật, hoặc dùng --provider ollama/custom nếu endpoint không cần key"
    )]
    MissingApiKey(String),
    #[error("base_url '{0}' không hợp lệ: phải là URL http(s) tuyệt đối, ví dụ https://api.groq.com/openai/v1")]
    InvalidBaseUrl(String),
    #[error("không tìm thấy provider id '{0}'. Xem danh sách bằng: tez provider list")]
    ProviderIdNotFound(String),
}

/// Các provider OpenAI-compatible được hỗ trợ sẵn.
pub const KNOWN_PROVIDERS: &[&str] =
    &["groq", "openrouter", "openai", "gemini", "ollama", "custom"];

/// Kiểm tra tên provider có nằm trong danh sách hỗ trợ.
pub fn is_known_provider(provider: &str) -> bool {
    let provider = provider.trim().to_lowercase();
    KNOWN_PROVIDERS.contains(&provider.as_str())
}

/// Chặn tên provider lạ để không âm thầm gửi key tới endpoint mặc định.
pub fn validate_provider(provider: &str) -> Result<(), ConnectionError> {
    if is_known_provider(provider) {
        Ok(())
    } else {
        Err(ConnectionError::ProviderNotFound(
            provider.trim().to_string(),
        ))
    }
}

/// Provider cloud bắt buộc có API key; ollama/custom có thể chạy không key.
pub fn requires_api_key(provider: &str) -> bool {
    matches!(
        provider.trim().to_lowercase().as_str(),
        "groq" | "openrouter" | "openai" | "gemini"
    )
}

/// Kiểm tra base_url là URL http(s) tuyệt đối có host.
pub fn validate_base_url(base_url: &str) -> Result<(), ConnectionError> {
    let url = base_url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| ConnectionError::InvalidBaseUrl(url.to_string()))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || url.chars().any(char::is_whitespace) {
        return Err(ConnectionError::InvalidBaseUrl(url.to_string()));
    }
    Ok(())
}

/// Tên hiển thị mặc định cho từng loại provider.
pub fn display_name(provider: &str) -> &'static str {
    match provider.trim().to_lowercase().as_str() {
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
    match provider.trim().to_lowercase().as_str() {
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
    match provider.trim().to_lowercase().as_str() {
        "groq" => "openai/gpt-oss-120b",
        "openrouter" => "meta-llama/llama-3.3-70b-instruct",
        "ollama" => "qwen3:4b",
        "gemini" => "gemini-3.5-flash",
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
    validate_provider(provider)?;
    let provider = provider.trim().to_lowercase();
    let base_url = match base_url.map(str::trim).filter(|value| !value.is_empty()) {
        Some(url) => {
            validate_base_url(url)?;
            url.to_string()
        }
        None if provider == "custom" => return Err(ConnectionError::MissingBaseUrl),
        None => default_base_url(&provider).to_string(),
    };
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_model(&provider).to_string());
    if requires_api_key(&provider) && api_key.trim().is_empty() {
        return Err(ConnectionError::MissingApiKey(provider));
    }

    // Giữ key cũ nếu master.key biến mất; tạo key mới sẽ phá mọi ciphertext cũ.
    if !dir.join("master.key").exists() {
        if let Some(existing) = load(dir)? {
            if !existing.encrypted_key.is_empty() {
                return Err(ConnectionError::Vault(crate::VaultError::MissingMasterKey));
            }
        }
    }
    let vault = Vault::load_or_create(dir)?;
    let encrypted_key = vault.encrypt(api_key.trim().as_bytes())?;
    let connection = ConnectionFile {
        provider,
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
    let vault = Vault::load(dir)?;
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
    fn rejects_unknown_provider() {
        let dir = tempfile::tempdir().unwrap();
        let error = save(dir.path(), "bogus", None, None, "sk-secret").unwrap_err();
        assert!(matches!(error, ConnectionError::ProviderNotFound(_)));
        assert!(!path(dir.path()).exists());

        assert!(is_known_provider("Groq"));
        assert!(is_known_provider(" custom "));
        assert!(!is_known_provider("bogus"));
        assert!(validate_provider("openai").is_ok());
    }

    #[test]
    fn rejects_empty_key_for_cloud_providers() {
        let dir = tempfile::tempdir().unwrap();
        let error = save(dir.path(), "groq", None, None, "   ").unwrap_err();
        assert!(matches!(error, ConnectionError::MissingApiKey(_)));
        assert!(!path(dir.path()).exists());

        assert!(save(dir.path(), "ollama", None, None, "").is_ok());
        assert!(save(
            dir.path(),
            "custom",
            Some("http://127.0.0.1:11434/v1"),
            None,
            ""
        )
        .is_ok());
    }

    #[test]
    fn detects_providers_that_need_key() {
        assert!(requires_api_key("Groq"));
        assert!(requires_api_key(" gemini "));
        assert!(!requires_api_key("ollama"));
        assert!(!requires_api_key("custom"));
    }

    #[test]
    fn normalizes_provider_case_before_defaults() {
        let dir = tempfile::tempdir().unwrap();
        // Tên viết hoa không được rơi về endpoint OpenAI.
        let saved = save(dir.path(), "OpenRouter", None, None, "sk-or-test").unwrap();
        assert_eq!(saved.provider, "openrouter");
        assert_eq!(saved.base_url, default_base_url("openrouter"));
        assert_eq!(saved.model, default_model("openrouter"));
        assert_ne!(saved.base_url, default_base_url("openai"));
        assert_eq!(default_model(" Groq "), "openai/gpt-oss-120b");
        assert_eq!(default_model("gemini"), "gemini-3.5-flash");
    }

    #[test]
    fn rejects_invalid_base_url() {
        let dir = tempfile::tempdir().unwrap();
        for url in [
            "api.groq.com",
            "ftp://example.com",
            "https://",
            "https://exa mple.com",
        ] {
            let error = save(dir.path(), "custom", Some(url), None, "").unwrap_err();
            assert!(
                matches!(error, ConnectionError::InvalidBaseUrl(_)),
                "url {url} phải bị từ chối"
            );
        }
        assert!(save(
            dir.path(),
            "custom",
            Some("https://gateway.example.com/v1?api-version=2024-01-01"),
            None,
            ""
        )
        .is_ok());
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
