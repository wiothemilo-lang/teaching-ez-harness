//! Ghi/đọc file cấu hình an toàn.
//!
//! Mỗi lần ghi dùng temp file cùng thư mục + fsync + rename để reader không
//! thấy nội dung dở dang. File cũ được giữ ở `<tên>.bak` và được dùng làm
//! fallback khi file chính không parse được.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) enum ReadError {
    Io(io::Error),
    Json(serde_json::Error),
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ReadError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bak");
    PathBuf::from(backup)
}

/// Ghi JSON nguyên tử. `is_valid` quyết định nội dung cũ có đáng giữ làm
/// backup hay không (tránh biến `.bak` thành bản sao của file đã hỏng).
pub(crate) fn write_json(
    path: &Path,
    bytes: &[u8],
    is_valid: impl Fn(&[u8]) -> bool,
) -> io::Result<()> {
    let backup = backup_path(path);
    match fs::read(path) {
        Ok(existing) => {
            if is_valid(&existing) {
                write_atomic(&backup, &existing)?;
            } else {
                // File chính hỏng: giữ backup cũ nếu nó còn đọc được, nếu không
                // dùng nội dung mới làm mốc để lần sau vẫn có fallback.
                let backup_is_valid = fs::read(&backup).ok().is_some_and(|bytes| is_valid(&bytes));
                if !backup_is_valid {
                    write_atomic(&backup, bytes)?;
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Lần ghi đầu tiên: vẫn tạo backup để có thể phục hồi sau này.
            write_atomic(&backup, bytes)?;
        }
        Err(error) => return Err(error),
    }
    write_atomic(path, bytes)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ReadError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };

    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Some(value)),
        Err(primary) => {
            let backup = backup_path(path);
            let backup_bytes = match fs::read(&backup) {
                Ok(bytes) => bytes,
                Err(_) => return Err(primary.into()),
            };
            match serde_json::from_slice(&backup_bytes) {
                Ok(value) => Ok(Some(value)),
                Err(_) => Err(primary.into()),
            }
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let temp = temp_path(path);
    let result = (|| {
        let mut file = create_private_new(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        sync_dir(parent)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn create_private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), counter))
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}
