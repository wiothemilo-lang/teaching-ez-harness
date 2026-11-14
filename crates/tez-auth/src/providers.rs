//! Sổ provider: nhiều kết nối đã lưu, một kết nối đang dùng.
//!
//! `providers.json` là nguồn chính; `connection.json` vẫn được ghi để CLI cũ
//! và các bản desktop trước đây đọc được. Key trong cả hai file đều là ciphertext.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::connection::{
    self, default_base_url, default_model, display_name, ConnectionError, ConnectionFile,
};
use crate::Vault;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub encrypted_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderBook {
    #[serde(default = "default_version")]
    pub version: u32,
    pub active: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
}

impl ProviderBook {
    pub fn find(&self, id: &str) -> Option<&ProviderEntry> {
        self.providers.iter().find(|entry| entry.id == id)
    }

    pub fn active(&self) -> Option<&ProviderEntry> {
        self.active.as_deref().and_then(|id| self.find(id))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderInput<'a> {
    pub id: Option<&'a str>,
    pub name: &'a str,
    pub provider: &'a str,
    pub base_url: Option<&'a str>,
    pub model: Option<&'a str>,
    /// `None` giữ key cũ (khi sửa); `Some("")` là provider không cần key.
    pub api_key: Option<&'a str>,
    pub set_active: bool,
}

fn default_version() -> u32 {
    1
}

pub fn path(dir: impl AsRef<Path>) -> PathBuf {
    dir.as_ref().join("providers.json")
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// Đọc sổ provider; nếu chưa có thì migrate từ `connection.json`.
pub fn load(dir: impl AsRef<Path>) -> Result<ProviderBook, ConnectionError> {
    let dir = dir.as_ref();
    let path = path(dir);

    if path.exists() {
        let bytes = std::fs::read(path)?;
        let mut book: ProviderBook = serde_json::from_slice(&bytes)?;
        if book.active.is_none() && !book.providers.is_empty() {
            book.active = Some(book.providers[0].id.clone());
        }
        return Ok(book);
    }

    if let Some(saved) = connection::load(dir)? {
        let id = saved.provider.clone();
        let entry = ProviderEntry {
            id: id.clone(),
            name: display_name(&saved.provider).to_string(),
            provider: saved.provider,
            base_url: saved.base_url,
            model: saved.model,
            encrypted_key: saved.encrypted_key,
        };
        return Ok(ProviderBook {
            version: 1,
            active: Some(id),
            providers: vec![entry],
        });
    }

    Ok(ProviderBook {
        version: 1,
        active: None,
        providers: Vec::new(),
    })
}

/// Ghi sổ provider và đồng bộ `connection.json` với provider đang dùng.
pub fn save(dir: impl AsRef<Path>, book: &ProviderBook) -> Result<(), ConnectionError> {
    let dir = dir.as_ref();
    let bytes = serde_json::to_vec_pretty(book)?;
    std::fs::write(path(dir), bytes)?;
    restrict_permissions(&path(dir))?;
    sync_connection(dir, book)?;
    Ok(())
}

fn sync_connection(dir: &Path, book: &ProviderBook) -> Result<(), ConnectionError> {
    match book.active() {
        Some(entry) => {
            let connection = ConnectionFile {
                provider: entry.provider.clone(),
                base_url: entry.base_url.clone(),
                model: entry.model.clone(),
                encrypted_key: entry.encrypted_key.clone(),
            };
            std::fs::write(
                connection::path(dir),
                serde_json::to_vec_pretty(&connection)?,
            )?;
            restrict_permissions(&connection::path(dir))?;
        }
        None => {
            let path = connection::path(dir);
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

/// Thêm mới hoặc cập nhật một provider. Trả về entry sau khi lưu.
pub fn upsert(
    dir: impl AsRef<Path>,
    input: ProviderInput<'_>,
) -> Result<ProviderEntry, ConnectionError> {
    let dir = dir.as_ref();
    let mut book = load(dir)?;
    let provider = input.provider.trim().to_lowercase();
    connection::validate_provider(&provider)?;
    let requested_id = input
        .id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let id = requested_id.unwrap_or_else(|| format!("{}-{}", slug(&provider), short_id()));
    let existing = book.find(&id).cloned();

    let base_url = match input
        .base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(url) => url.to_string(),
        None if provider == "custom" => return Err(ConnectionError::MissingBaseUrl),
        None => existing
            .as_ref()
            .map(|entry| entry.base_url.clone())
            .unwrap_or_else(|| default_base_url(&provider).to_string()),
    };
    let model = input
        .model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| existing.as_ref().map(|entry| entry.model.clone()))
        .unwrap_or_else(|| default_model(&provider).to_string());
    let trimmed_name = input.name.trim();
    let name = if trimmed_name.is_empty() {
        display_name(&provider).to_string()
    } else {
        trimmed_name.to_string()
    };
    let encrypted_key = match input.api_key {
        Some(key) => Vault::load_or_create(dir)?.encrypt(key.as_bytes())?,
        None => match existing.as_ref() {
            Some(entry) => entry.encrypted_key.clone(),
            None => Vault::load_or_create(dir)?.encrypt(b"")?,
        },
    };

    let entry = ProviderEntry {
        id: id.clone(),
        name,
        provider,
        base_url,
        model,
        encrypted_key,
    };

    match book.providers.iter().position(|item| item.id == id) {
        Some(index) => book.providers[index] = entry.clone(),
        None => book.providers.push(entry.clone()),
    }
    if input.set_active || book.active.is_none() {
        book.active = Some(id);
    }
    save(dir, &book)?;
    Ok(entry)
}

pub fn set_active(dir: impl AsRef<Path>, id: &str) -> Result<ProviderEntry, ConnectionError> {
    let dir = dir.as_ref();
    let mut book = load(dir)?;
    if book.find(id).is_none() {
        return Err(ConnectionError::ProviderNotFound(id.to_string()));
    }
    book.active = Some(id.to_string());
    save(dir, &book)?;
    Ok(book.find(id).cloned().expect("checked above"))
}

pub fn remove(dir: impl AsRef<Path>, id: &str) -> Result<bool, ConnectionError> {
    let dir = dir.as_ref();
    let mut book = load(dir)?;
    let before = book.providers.len();
    book.providers.retain(|entry| entry.id != id);
    if book.providers.len() == before {
        return Ok(false);
    }
    if book.active.as_deref() == Some(id) {
        book.active = book.providers.first().map(|entry| entry.id.clone());
    }
    save(dir, &book)?;
    Ok(true)
}

/// Provider đang dùng, đã migrate từ `connection.json` nếu cần.
pub fn active(dir: impl AsRef<Path>) -> Result<Option<ProviderEntry>, ConnectionError> {
    Ok(load(dir)?.active().cloned())
}

pub fn decrypt_key(
    dir: impl AsRef<Path>,
    entry: &ProviderEntry,
) -> Result<String, ConnectionError> {
    let vault = Vault::load_or_create(dir)?;
    let bytes = vault.decrypt(&entry.encrypted_key)?;
    String::from_utf8(bytes).map_err(|_| ConnectionError::InvalidKeyEncoding)
}

fn slug(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "provider".to_string()
    } else {
        trimmed.to_string()
    }
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
    fn migrates_legacy_connection() {
        let dir = tempfile::tempdir().unwrap();
        connection::save(dir.path(), "groq", None, None, "sk-old").unwrap();

        let book = load(dir.path()).unwrap();
        assert_eq!(book.providers.len(), 1);
        assert_eq!(book.active.as_deref(), Some("groq"));
        assert_eq!(book.providers[0].name, "Groq");
    }

    #[test]
    fn adds_multiple_providers_and_switches_active() {
        let dir = tempfile::tempdir().unwrap();
        let groq = upsert(
            dir.path(),
            ProviderInput {
                name: "Groq",
                provider: "groq",
                api_key: Some("sk-groq"),
                set_active: true,
                ..Default::default()
            },
        )
        .unwrap();
        let ollama = upsert(
            dir.path(),
            ProviderInput {
                name: "Ollama local",
                provider: "ollama",
                api_key: Some(""),
                set_active: false,
                ..Default::default()
            },
        )
        .unwrap();

        let book = load(dir.path()).unwrap();
        assert_eq!(book.providers.len(), 2);
        assert_eq!(book.active.as_deref(), Some(groq.id.as_str()));
        assert_eq!(decrypt_key(dir.path(), &groq).unwrap(), "sk-groq");
        assert_eq!(decrypt_key(dir.path(), &ollama).unwrap(), "");

        set_active(dir.path(), &ollama.id).unwrap();
        assert_eq!(
            connection::load(dir.path()).unwrap().unwrap().model,
            default_model("ollama")
        );
    }

    #[test]
    fn keeps_key_when_editing_without_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let saved = upsert(
            dir.path(),
            ProviderInput {
                name: "Groq",
                provider: "groq",
                api_key: Some("sk-keep"),
                set_active: true,
                ..Default::default()
            },
        )
        .unwrap();

        let edited = upsert(
            dir.path(),
            ProviderInput {
                id: Some(&saved.id),
                name: "Groq nhanh",
                provider: "groq",
                model: Some("llama-3.1-8b-instant"),
                api_key: None,
                set_active: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(edited.name, "Groq nhanh");
        assert_eq!(edited.model, "llama-3.1-8b-instant");
        assert_eq!(decrypt_key(dir.path(), &edited).unwrap(), "sk-keep");
    }

    #[test]
    fn rejects_unknown_provider_name() {
        let dir = tempfile::tempdir().unwrap();
        let error = upsert(
            dir.path(),
            ProviderInput {
                name: "",
                provider: "bogus",
                api_key: Some("sk-secret"),
                set_active: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            connection::ConnectionError::ProviderNotFound(_)
        ));
        assert!(!path(dir.path()).exists());
        assert!(load(dir.path()).unwrap().providers.is_empty());
    }

    #[test]
    fn removing_active_provider_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let first = upsert(
            dir.path(),
            ProviderInput {
                name: "First",
                provider: "groq",
                api_key: Some("a"),
                set_active: true,
                ..Default::default()
            },
        )
        .unwrap();
        let second = upsert(
            dir.path(),
            ProviderInput {
                name: "Second",
                provider: "ollama",
                api_key: Some(""),
                set_active: false,
                ..Default::default()
            },
        )
        .unwrap();

        remove(dir.path(), &first.id).unwrap();
        let book = load(dir.path()).unwrap();
        assert_eq!(book.active.as_deref(), Some(second.id.as_str()));
    }
}
