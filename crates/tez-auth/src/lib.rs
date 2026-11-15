pub mod connection;
pub mod providers;

mod storage;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    #[error(
        "không tìm thấy master.key trong thư mục dữ liệu — không thể giải mã API key đã lưu. \
         Khôi phục master.key từ bản sao lưu; nếu không còn bản sao lưu, hãy sao lưu rồi xóa \
         providers.json (và connection.json) để tạo vault mới, sau đó nhập lại API key bằng \
         'tez connect ...'"
    )]
    MissingMasterKey,
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

impl std::fmt::Debug for Vault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vault")
            .field("key", &"[redacted]")
            .field("key_path", &self.key_path)
            .finish()
    }
}

impl Vault {
    pub fn load_or_create(dir: impl AsRef<Path>) -> Result<Self, VaultError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let key_path = dir.join("master.key");

        let mut generated = [0u8; 32];
        OsRng.fill_bytes(&mut generated);

        // `create_new` là bước nguyên tử: chỉ một process thắng. Không dùng
        // exists()+write vì write sẽ truncate key của process khác.
        let key = match create_private_file(&key_path) {
            Ok(mut file) => {
                file.write_all(&generated)?;
                file.sync_all()?;
                drop(file);
                sync_dir(dir)?;
                generated
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                read_existing_key(&key_path)?
            }
            Err(error) => return Err(error.into()),
        };

        Ok(Self { key, key_path })
    }

    /// Mở vault đã tồn tại, không tạo master.key mới.
    ///
    /// Dùng khi giải mã dữ liệu cũ: tạo key mới sẽ khiến ciphertext cũ vĩnh viễn
    /// không đọc được và che mất nguyên nhân thật.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, VaultError> {
        let dir = dir.as_ref();
        let key_path = dir.join("master.key");
        let key = read_existing_key(&key_path)?;
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

fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Đọc key đã có. Retry ngắn để chịu được cửa sổ giữa `create_new` và `write`
/// của process khác; tuyệt đối không truncate hay tạo lại file.
fn read_existing_key(path: &Path) -> Result<[u8; 32], VaultError> {
    const ATTEMPTS: u32 = 100;
    const RETRY_DELAY: Duration = Duration::from_millis(10);

    for attempt in 0..ATTEMPTS {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() == 32 => {
                // File cũ có thể được tạo trước khi siết quyền: luôn đặt lại 0600.
                set_private_permissions(path)?;
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                return Ok(key);
            }
            Ok(_) if attempt + 1 < ATTEMPTS => std::thread::sleep(RETRY_DELAY),
            Ok(_) => return Err(VaultError::InvalidKey),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::MissingMasterKey)
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(VaultError::InvalidKey)
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<(), VaultError> {
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<(), VaultError> {
    Ok(())
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

    #[test]
    fn concurrent_load_or_create_uses_one_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let mut handles = Vec::new();

        for _ in 0..16 {
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let vault = Vault::load_or_create(&path).unwrap();
                vault.encrypt(b"shared-secret").unwrap()
            }));
        }

        let ciphertexts: Vec<String> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let vault = Vault::load(dir.path()).unwrap();
        for ciphertext in ciphertexts {
            assert_eq!(vault.decrypt(&ciphertext).unwrap(), b"shared-secret");
        }
    }

    #[test]
    fn load_opens_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let created = Vault::load_or_create(dir.path()).unwrap();
        let loaded = Vault::load(dir.path()).unwrap();

        let encrypted = created.encrypt(b"secret").unwrap();
        assert_eq!(loaded.decrypt(&encrypted).unwrap(), b"secret");
    }

    #[test]
    fn load_or_create_does_not_rewrite_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        let key_path = dir.path().join("master.key");
        std::fs::write(&key_path, key).unwrap();

        let vault = Vault::load_or_create(dir.path()).unwrap();
        let encrypted = vault.encrypt(b"secret").unwrap();
        assert_eq!(vault.decrypt(&encrypted).unwrap(), b"secret");
        assert_eq!(std::fs::read(&key_path).unwrap(), key);
    }

    #[test]
    fn load_never_creates_a_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let error = Vault::load(dir.path()).unwrap_err();

        assert!(matches!(error, VaultError::MissingMasterKey));
        assert!(!dir.path().join("master.key").exists());
        let message = error.to_string();
        assert!(message.contains("master.key"));
        assert!(message.contains("providers.json"));
    }

    #[cfg(unix)]
    #[test]
    fn master_key_is_private_from_creation() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::load_or_create(dir.path()).unwrap();
        let mode = std::fs::metadata(vault.key_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
