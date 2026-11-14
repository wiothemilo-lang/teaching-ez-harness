pub mod connection;
pub mod providers;

use std::path::{Path, PathBuf};

use aes_gcm::aead::{rand_core::RngCore, Aead, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid master key length")]
    InvalidKey,
    #[error("crypto error")]
    Crypto,
    #[error("invalid ciphertext format")]
    InvalidFormat,
    #[error("base64: {0}")]
    Base64(#[from] base64::DecodeError),
}

/// MVP vault: master key file 0600 trong app data.
///
/// TODO: thay bằng OS keychain (`keyring` crate) khi build desktop/Tauri.
pub struct Vault {
    key: [u8; 32],
    key_path: PathBuf,
}

impl Vault {
    pub fn load_or_create(dir: impl AsRef<Path>) -> Result<Self, VaultError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let key_path = dir.join("master.key");

        let key = if key_path.exists() {
            let bytes = std::fs::read(&key_path)?;
            if bytes.len() != 32 {
                return Err(VaultError::InvalidKey);
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            key
        } else {
            let mut key = [0u8; 32];
            OsRng.fill_bytes(&mut key);
            std::fs::write(&key_path, key)?;
            set_private_permissions(&key_path)?;
            key
        };

        Ok(Self { key, key_path })
    }

    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    /// Trả về chuỗi `v1:<iv_b64>:<ciphertext_b64>`.
    pub fn encrypt(&self, plain: &[u8]) -> Result<String, VaultError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| VaultError::Crypto)?;
        let mut iv = [0u8; 12];
        OsRng.fill_bytes(&mut iv);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&iv), plain)
            .map_err(|_| VaultError::Crypto)?;

        Ok(format!(
            "v1:{}:{}",
            STANDARD.encode(iv),
            STANDARD.encode(ciphertext)
        ))
    }

    pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, VaultError> {
        let mut parts = encoded.split(':');
        let version = parts.next().ok_or(VaultError::InvalidFormat)?;
        if version != "v1" {
            return Err(VaultError::InvalidFormat);
        }
        let iv = STANDARD.decode(parts.next().ok_or(VaultError::InvalidFormat)?)?;
        let ciphertext = STANDARD.decode(parts.next().ok_or(VaultError::InvalidFormat)?)?;
        if iv.len() != 12 {
            return Err(VaultError::InvalidFormat);
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| VaultError::Crypto)?;
        cipher
            .decrypt(Nonce::from_slice(&iv), ciphertext.as_ref())
            .map_err(|_| VaultError::Crypto)
    }
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), VaultError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), VaultError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::load_or_create(dir.path()).unwrap();
        let secret = b"sk-test-123";

        let encrypted = vault.encrypt(secret).unwrap();
        assert!(encrypted.starts_with("v1:"));
        assert_eq!(vault.decrypt(&encrypted).unwrap(), secret);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::load_or_create(dir.path()).unwrap();
        let encrypted = vault.encrypt(b"secret").unwrap();
        let parts: Vec<&str> = encrypted.split(':').collect();
        let bad = format!("{}:{}:{}", parts[0], parts[1], "AAAA");

        assert!(vault.decrypt(&bad).is_err());
    }
}
