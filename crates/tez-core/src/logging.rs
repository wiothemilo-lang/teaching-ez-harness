//! Log ứng dụng bền vững ra `logs/tez.log`.
//!
//! Desktop thường được mở từ GUI nên `stderr` không hiển thị. Module này ghi
//! lỗi, panic và các mốc khởi động ra file để người dùng vẫn có dấu vết khi
//! ứng dụng thoát bất thường. File được xoay sang `tez.log.1` khi vượt 5 MB;
//! `scripts/disk-guard.sh` tiếp tục dọn các file log cũ theo hạn mức.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const LOG_DIR: &str = "logs";
const LOG_FILE: &str = "tez.log";
const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();
static PANIC_HOOK: Once = Once::new();

/// Logger file đơn giản, an toàn khi nhiều thread cùng ghi.
#[derive(Debug)]
pub struct FileLogger {
    path: PathBuf,
    max_bytes: u64,
    component: String,
    lock: Mutex<()>,
}

impl FileLogger {
    pub fn new(path: impl Into<PathBuf>, component: impl Into<String>) -> io::Result<Self> {
        Self::with_max_bytes(path, component, DEFAULT_MAX_BYTES)
    }

    pub fn with_max_bytes(
        path: impl Into<PathBuf>,
        component: impl Into<String>,
        max_bytes: u64,
    ) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        Ok(Self {
            path,
            max_bytes: max_bytes.max(1),
            component: component.into(),
            lock: Mutex::new(()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, level: &str, message: &str) -> io::Result<()> {
        // Một panic hook có thể chạy khi mutex đang bị poison; vẫn cố ghi nốt.
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        self.rotate_if_needed()?;

        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        writeln!(
            file,
            "{} [{}] [{}] {}",
            timestamp(),
            self.component,
            level,
            message
        )?;
        file.flush()?;
        set_private_permissions(&self.path);
        Ok(())
    }

    fn rotate_if_needed(&self) -> io::Result<()> {
        let too_large = fs::metadata(&self.path)
            .map(|metadata| metadata.len() >= self.max_bytes)
            .unwrap_or(false);
        if !too_large {
            return Ok(());
        }

        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(LOG_FILE);
        let rotated = self.path.with_file_name(format!("{name}.1"));
        // Windows không cho rename đè file đã tồn tại.
        let _ = fs::remove_file(&rotated);
        match fs::rename(&self.path, &rotated) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Khởi tạo logger toàn cục và panic hook. Gọi một lần lúc ứng dụng bắt đầu.
///
/// Trả về đường dẫn file log. Nếu logger đã được khởi tạo, lần gọi sau không
/// thay đổi đích ghi.
pub fn init(data_dir: impl AsRef<Path>, component: &str) -> io::Result<PathBuf> {
    let path = log_path(data_dir);
    let logger = FileLogger::new(&path, component)?;
    let path = logger.path().to_path_buf();
    let _ = LOGGER.set(logger);
    install_panic_hook();
    Ok(path)
}

pub fn log_path(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join(LOG_DIR).join(LOG_FILE)
}

pub fn info(message: impl AsRef<str>) {
    write("INFO", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    write("WARN", message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    write("ERROR", message.as_ref());
}

pub fn debug(message: impl AsRef<str>) {
    write("DEBUG", message.as_ref());
}

pub fn write(level: &str, message: &str) {
    if let Some(logger) = LOGGER.get() {
        let _ = logger.write(level, message);
    }
}

/// RFC 3339 UTC, dùng chung cho log và tên bản sao lưu.
pub fn timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>");
            let location = info
                .location()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file(),
                        location.line(),
                        location.column()
                    )
                })
                .unwrap_or_else(|| "<unknown>".to_string());
            let message = panic_message(info);
            let backtrace = std::backtrace::Backtrace::force_capture();
            write(
                "PANIC",
                &format!("thread '{name}' panicked at {location}: {message}\n{backtrace}"),
            );
            previous(info);
        }));
    });
}

fn panic_message<'a>(info: &'a PanicHookInfo<'_>) -> &'a str {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        message
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.as_str()
    } else {
        "<non-string panic payload>"
    }
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_rotates_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/tez.log");
        let logger = FileLogger::with_max_bytes(&path, "test", 1).unwrap();

        logger.write("INFO", "dòng đầu").unwrap();
        assert!(path.exists());
        logger.write("ERROR", "dòng hai").unwrap();

        let current = fs::read_to_string(&path).unwrap();
        let rotated = fs::read_to_string(dir.path().join("logs/tez.log.1")).unwrap();
        assert!(current.contains("dòng hai"));
        assert!(rotated.contains("dòng đầu"));
    }

    #[test]
    fn timestamp_is_rfc3339() {
        let value = timestamp();
        assert!(value.ends_with('Z') || value.contains("+00:00"));
        assert!(value.contains('T'));
    }
}
