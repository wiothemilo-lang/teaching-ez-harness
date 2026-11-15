//! Sao lưu/phục hồi dữ liệu local của tez.
//!
//! Một bản sao lưu là một thư mục tự chứa:
//! - `tez.db` được chụp bằng SQLite backup API (nhất quán cả WAL);
//! - các file cấu hình và vault (`providers.json`, `master.key`, ...);
//! - skill, plugin, memory, plan, session artifact và ảnh đính kèm;
//! - `manifest.json` chứa kích thước + SHA-256 để kiểm tra trước khi phục hồi.
//!
//! Không đưa `workspaces/`, `logs/`, `updates/` và chính thư mục `backups/`
//! vào bản sao lưu. `approvals.json` cũng bị loại vì đó là quyền tạm thời,
//! không nên sống lại sau khi phục hồi.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tez_store::Store;
use thiserror::Error;
use time::OffsetDateTime;

pub const FORMAT_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";

const CONFIG_FILES: &[&str] = &[
    "providers.json",
    "providers.json.bak",
    "connection.json",
    "connection.json.bak",
    "mcp.json",
    "mcp.json.bak",
    "harness.json",
    "master.key",
    "oauth.json",
    "audit.jsonl",
    "audit.jsonl.1",
];

const DATA_DIRS: &[&str] = &["skills", "plugins", "memory", "plans", "sessions", "images"];

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("sqlite: {0}")]
    Store(#[from] tez_store::StoreError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bản sao lưu không hợp lệ: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub app_version: String,
    pub created_at_unix: u64,
    pub db_integrity: Option<String>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct BackupReport {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub skipped: Vec<String>,
    pub db_integrity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub safety_backup: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub app_version: String,
    pub created_at_unix: u64,
    pub files: usize,
    pub bytes: u64,
}

/// Tạo bản sao lưu mới tại `destination`. Thư mục đích phải chưa tồn tại.
pub fn create(data_dir: &Path, destination: &Path) -> Result<BackupReport, BackupError> {
    if destination.exists() {
        return Err(BackupError::Invalid(format!(
            "thư mục đích đã tồn tại: {}",
            destination.display()
        )));
    }
    if let Some(parent) = destination.parent() {
        create_private_dir(parent)?;
    }
    create_private_dir(destination)?;

    let result = create_inner(data_dir, destination);
    if result.is_err() {
        // Không để lại thư mục dở dang khiến lần sau tưởng là bản hợp lệ.
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn create_inner(data_dir: &Path, destination: &Path) -> Result<BackupReport, BackupError> {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut db_integrity = None;

    let db_path = data_dir.join("tez.db");
    if db_path.exists() {
        let store = Store::open(&db_path)?;
        let integrity = store.quick_check()?;
        db_integrity = Some(integrity.clone());
        if integrity != "ok" {
            return Err(BackupError::Invalid(format!(
                "DB không vượt qua quick_check: {integrity}"
            )));
        }
        let target = destination.join("tez.db");
        store.backup_to(&target)?;
        drop(store);
        let metadata = fs::metadata(&target)?;
        entries.push(FileEntry {
            path: "tez.db".to_string(),
            bytes: metadata.len(),
            sha256: sha256_file(&target)?,
        });
    }

    for name in CONFIG_FILES {
        add_existing_file(data_dir, destination, Path::new(name), &mut entries)?;
    }

    for name in DATA_DIRS {
        let source = data_dir.join(name);
        if source.exists() {
            copy_tree(data_dir, &source, destination, &mut entries, &mut skipped)?;
        }
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at_unix: unix_now(),
        db_integrity,
        files: entries,
        skipped: skipped.clone(),
    };
    write_manifest(destination, &manifest)?;

    let bytes = manifest.files.iter().map(|entry| entry.bytes).sum();
    Ok(BackupReport {
        path: destination.to_path_buf(),
        files: manifest.files.len(),
        bytes,
        skipped,
        db_integrity: manifest.db_integrity,
    })
}

/// Phục hồi bản sao lưu vào `data_dir`.
///
/// Trước khi ghi đè, hàm tạo một bản sao lưu an toàn của dữ liệu hiện tại vào
/// `safety_root` (nếu đã có dữ liệu). Mọi file trong manifest được kiểm tra
/// kích thước + SHA-256 trước khi chạm vào dữ liệu thật.
pub fn restore(
    backup: &Path,
    data_dir: &Path,
    safety_root: &Path,
) -> Result<RestoreReport, BackupError> {
    let manifest = read_manifest(backup)?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(BackupError::Invalid(format!(
            "định dạng {} không được hỗ trợ (cần {FORMAT_VERSION})",
            manifest.format_version
        )));
    }
    verify_manifest(backup, &manifest)?;

    let safety_backup = if has_user_data(data_dir) {
        let destination = unique_path(safety_root, "pre-restore");
        Some(create(data_dir, &destination)?.path)
    } else {
        None
    };

    for entry in &manifest.files {
        let relative = Path::new(&entry.path);
        validate_relative(relative)?;
        let target = data_dir.join(relative);
        if entry.path == "tez.db" {
            // WAL/SHM cũ thuộc DB trước khi phục hồi; giữ lại sẽ làm hỏng DB mới.
            remove_if_exists(&data_dir.join("tez.db-wal"))?;
            remove_if_exists(&data_dir.join("tez.db-shm"))?;
        }
        replace_private(&backup.join(relative), &target)?;
    }

    if manifest.files.iter().any(|entry| entry.path == "tez.db") {
        let store = Store::open_read_only(data_dir.join("tez.db"))?;
        let integrity = store.quick_check()?;
        if integrity != "ok" {
            return Err(BackupError::Invalid(format!(
                "DB sau phục hồi không vượt qua quick_check: {integrity}"
            )));
        }
    }

    let bytes = manifest.files.iter().map(|entry| entry.bytes).sum();
    Ok(RestoreReport {
        path: backup.to_path_buf(),
        files: manifest.files.len(),
        bytes,
        safety_backup,
    })
}

pub fn list(backup_root: &Path) -> Result<Vec<BackupInfo>, BackupError> {
    let mut backups = Vec::new();
    let entries = match fs::read_dir(backup_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(backups),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let manifest = match read_manifest(&path) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        backups.push(BackupInfo {
            path,
            app_version: manifest.app_version,
            created_at_unix: manifest.created_at_unix,
            files: manifest.files.len(),
            bytes: manifest.files.iter().map(|file| file.bytes).sum(),
        });
    }
    backups.sort_by_key(|left| std::cmp::Reverse(left.created_at_unix));
    Ok(backups)
}

/// Đích mặc định: `<data>/backups/tez-backup-YYYYMMDD-HHMMSS`.
pub fn default_destination(data_dir: &Path) -> PathBuf {
    unique_path(&data_dir.join("backups"), "tez-backup")
}

pub fn manifest_path(backup: &Path) -> PathBuf {
    backup.join(MANIFEST_FILE)
}

fn read_manifest(backup: &Path) -> Result<Manifest, BackupError> {
    let path = manifest_path(backup);
    let bytes = fs::read(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            BackupError::Invalid(format!(
                "không thấy {} — đây không phải bản sao lưu tez",
                path.display()
            ))
        } else {
            error.into()
        }
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_manifest(destination: &Path, manifest: &Manifest) -> Result<(), BackupError> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    let path = manifest_path(destination);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn verify_manifest(backup: &Path, manifest: &Manifest) -> Result<(), BackupError> {
    for entry in &manifest.files {
        let relative = Path::new(&entry.path);
        validate_relative(relative)?;
        let path = backup.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                BackupError::Invalid(format!("thiếu file trong bản sao lưu: {}", entry.path))
            } else {
                error.into()
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(BackupError::Invalid(format!(
                "không phải file thường: {}",
                entry.path
            )));
        }
        if metadata.len() != entry.bytes {
            return Err(BackupError::Invalid(format!(
                "kích thước sai: {} (manifest {}, thực tế {})",
                entry.path,
                entry.bytes,
                metadata.len()
            )));
        }
        let hash = sha256_file(&path)?;
        if hash != entry.sha256 {
            return Err(BackupError::Invalid(format!(
                "SHA-256 không khớp: {}",
                entry.path
            )));
        }
    }

    if let Some(entry) = manifest.files.iter().find(|entry| entry.path == "tez.db") {
        let store = Store::open_read_only(backup.join(&entry.path))?;
        let integrity = store.quick_check()?;
        if integrity != "ok" {
            return Err(BackupError::Invalid(format!(
                "DB trong bản sao lưu hỏng: {integrity}"
            )));
        }
    }
    Ok(())
}

fn add_existing_file(
    data_dir: &Path,
    destination: &Path,
    relative: &Path,
    entries: &mut Vec<FileEntry>,
) -> Result<(), BackupError> {
    let source = data_dir.join(relative);
    if !source.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&source)?;
    if !metadata.file_type().is_file() {
        return Ok(());
    }
    let target = destination.join(relative);
    copy_private(&source, &target)?;
    entries.push(FileEntry {
        path: path_string(relative)?,
        bytes: metadata.len(),
        sha256: sha256_file(&target)?,
    });
    Ok(())
}

fn copy_tree(
    root: &Path,
    current: &Path,
    destination: &Path,
    entries: &mut Vec<FileEntry>,
    skipped: &mut Vec<String>,
) -> Result<(), BackupError> {
    let mut children = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        let path = child.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            skipped.push(path_string(relative)?);
            continue;
        }
        if file_type.is_dir() {
            let target = destination.join(relative);
            create_private_dir(&target)?;
            copy_tree(root, &path, destination, entries, skipped)?;
            continue;
        }
        if !file_type.is_file() {
            skipped.push(path_string(relative)?);
            continue;
        }

        let target = destination.join(relative);
        copy_private(&path, &target)?;
        entries.push(FileEntry {
            path: path_string(relative)?,
            bytes: metadata.len(),
            sha256: sha256_file(&target)?,
        });
    }
    Ok(())
}

fn path_string(path: &Path) -> Result<String, BackupError> {
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| {
            BackupError::Invalid(format!("tên file không phải UTF-8: {}", path.display()))
        })
}

fn validate_relative(path: &Path) -> Result<(), BackupError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(BackupError::Invalid(format!(
            "đường dẫn không hợp lệ: {}",
            path.display()
        )));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BackupError::Invalid(format!(
            "đường dẫn có thành phần không an toàn: {}",
            path.display()
        )));
    }
    Ok(())
}

fn copy_private(source: &Path, target: &Path) -> Result<(), BackupError> {
    if let Some(parent) = target.parent() {
        create_private_dir(parent)?;
    }
    let mut input = File::open(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(target)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    set_private_permissions(target);
    Ok(())
}

fn replace_private(source: &Path, target: &Path) -> Result<(), BackupError> {
    if let Some(parent) = target.parent() {
        create_private_dir(parent)?;
    }
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restore");
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = target.with_file_name(format!(
        ".{name}.restore.{}.{}",
        std::process::id(),
        counter
    ));
    copy_private(source, &temp)?;

    #[cfg(windows)]
    {
        remove_if_exists(target)?;
    }
    match fs::rename(&temp, target) {
        Ok(()) => {
            set_private_permissions(target);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error.into())
        }
    }
}

fn sha256_file(path: &Path) -> Result<String, BackupError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok(hex)
}

fn has_user_data(data_dir: &Path) -> bool {
    const MARKERS: &[&str] = &[
        "tez.db",
        "providers.json",
        "connection.json",
        "master.key",
        "mcp.json",
        "harness.json",
        "skills",
        "plugins",
        "memory",
    ];
    MARKERS.iter().any(|name| data_dir.join(name).exists())
}

fn create_private_dir(path: &Path) -> Result<(), BackupError> {
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

fn remove_if_exists(path: &Path) -> Result<(), BackupError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn unique_path(root: &Path, prefix: &str) -> PathBuf {
    let stamp = timestamp_for_name();
    let first = root.join(format!("{prefix}-{stamp}"));
    if !first.exists() {
        return first;
    }
    for index in 2..1000 {
        let candidate = root.join(format!("{prefix}-{stamp}-{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    root.join(format!("{prefix}-{stamp}-{}", std::process::id()))
}

fn timestamp_for_name() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tez_store::NewMessage;

    fn sample_data(dir: &Path) {
        let store = Store::open(dir.join("tez.db")).unwrap();
        store.ensure_profile("default", "Test").unwrap();
        let session = store
            .create_session_mode("default", None, None, "chat")
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &session,
                role: "user",
                content: "xin chào",
                model: None,
                tokens_in: 1,
                tokens_out: 2,
            })
            .unwrap();
        drop(store);

        fs::write(dir.join("providers.json"), "{\"version\":1}").unwrap();
        fs::write(dir.join("master.key"), [7u8; 32]).unwrap();
        fs::create_dir_all(dir.join("skills/demo")).unwrap();
        fs::write(dir.join("skills/demo/SKILL.md"), "# Demo").unwrap();
        fs::create_dir_all(dir.join("images")).unwrap();
        fs::write(dir.join("images/a.png"), b"png").unwrap();
        fs::write(dir.join("approvals.json"), "{\"entries\":[]}").unwrap();
        fs::create_dir_all(dir.join("workspaces")).unwrap();
        fs::write(dir.join("workspaces/keep.txt"), "không sao lưu").unwrap();
    }

    #[test]
    fn creates_and_restores_backup() {
        let source = tempfile::tempdir().unwrap();
        sample_data(source.path());
        let backup_root = tempfile::tempdir().unwrap();
        let destination = backup_root.path().join("backup");

        let report = create(source.path(), &destination).unwrap();
        assert!(report.files >= 5);
        assert_eq!(report.db_integrity.as_deref(), Some("ok"));
        assert!(destination.join("manifest.json").exists());
        assert!(destination.join("tez.db").exists());
        assert!(destination.join("skills/demo/SKILL.md").exists());
        assert!(!destination.join("approvals.json").exists());
        assert!(!destination.join("workspaces").exists());

        // Xoá dữ liệu hiện tại rồi phục hồi.
        fs::remove_file(source.path().join("tez.db")).unwrap();
        fs::remove_file(source.path().join("master.key")).unwrap();
        fs::remove_dir_all(source.path().join("skills")).unwrap();

        let restored = restore(&destination, source.path(), backup_root.path()).unwrap();
        assert!(restored.files >= 5);
        assert!(restored.safety_backup.is_some());
        assert!(source.path().join("master.key").exists());
        assert!(source.path().join("skills/demo/SKILL.md").exists());

        let store = Store::open(source.path().join("tez.db")).unwrap();
        assert_eq!(store.session_count().unwrap(), 1);
        assert_eq!(store.quick_check().unwrap(), "ok");
    }

    #[test]
    fn rejects_path_traversal_in_manifest() {
        let source = tempfile::tempdir().unwrap();
        sample_data(source.path());
        let backup_root = tempfile::tempdir().unwrap();
        let destination = backup_root.path().join("backup");
        create(source.path(), &destination).unwrap();

        let mut manifest = read_manifest(&destination).unwrap();
        manifest.files.push(FileEntry {
            path: "../escape.txt".to_string(),
            bytes: 1,
            sha256: "00".to_string(),
        });
        fs::write(
            manifest_path(&destination),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let error = restore(&destination, source.path(), backup_root.path()).unwrap_err();
        assert!(error.to_string().contains("không an toàn"));
        assert!(!source.path().parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn rejects_checksum_mismatch() {
        let source = tempfile::tempdir().unwrap();
        sample_data(source.path());
        let backup_root = tempfile::tempdir().unwrap();
        let destination = backup_root.path().join("backup");
        create(source.path(), &destination).unwrap();

        fs::write(destination.join("master.key"), [9u8; 32]).unwrap();
        let error = restore(&destination, source.path(), backup_root.path()).unwrap_err();
        assert!(error.to_string().contains("SHA-256"));
    }

    #[test]
    fn lists_backups_newest_first() {
        let source = tempfile::tempdir().unwrap();
        sample_data(source.path());
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("a");
        create(source.path(), &first).unwrap();
        let second = root.path().join("b");
        create(source.path(), &second).unwrap();

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|entry| entry.path == first));
        assert!(listed.iter().any(|entry| entry.path == second));
    }
}
