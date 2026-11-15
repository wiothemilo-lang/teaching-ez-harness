//! Sandbox cho `run_command` trên Linux bằng bubblewrap.
//!
//! Mục tiêu: lệnh do agent chạy không đọc được secret của app (`master.key`,
//! `providers.json`, ...) và không tự do gửi dữ liệu ra mạng. Đây là lớp bảo vệ
//! bổ sung cho permission pipeline, không thay thế việc chạy app bằng tài khoản
//! ít quyền.
//!
//! Sandbox dùng mount namespace + network namespace của bubblewrap. Nếu thiếu
//! `bwrap`, lệnh bị từ chối thay vì âm thầm chạy không sandbox.

use std::path::{Path, PathBuf};

use tokio::process::Command;

/// Cấu hình sandbox đọc từ `harness.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxSettings {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Cho phép lệnh dùng mạng. Mặc định tắt để chặn exfiltration.
    #[serde(default)]
    pub allow_network: bool,
}

fn default_enabled() -> bool {
    cfg!(target_os = "linux")
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            allow_network: false,
        }
    }
}

/// Chính sách đã gắn với data dir của app.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    pub enabled: bool,
    pub allow_network: bool,
    pub data_dir: PathBuf,
}

impl SandboxPolicy {
    pub fn from_settings(settings: &SandboxSettings, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            enabled: settings.enabled,
            allow_network: settings.allow_network,
            data_dir: data_dir.into(),
        }
    }
}

/// Tìm bubblewrap: ưu tiên `TEZ_BWRAP`, sau đó `bwrap` trên PATH.
pub fn bwrap_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TEZ_BWRAP") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("bwrap"))
        .find(|candidate| candidate.is_file())
}

/// Kiểm tra nhanh bubblewrap có chạy được trong môi trường hiện tại không.
pub fn probe() -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") {
        return Err("sandbox hiện chỉ hỗ trợ Linux".to_string());
    }
    let bwrap = bwrap_path().ok_or_else(|| {
        "không tìm thấy bubblewrap (cài gói `bubblewrap` hoặc đặt TEZ_BWRAP)".to_string()
    })?;
    let status = std::process::Command::new(&bwrap)
        .args(["--ro-bind", "/", "/", "--unshare-net", "--", "/bin/true"])
        .status()
        .map_err(|error| format!("không chạy được {}: {error}", bwrap.display()))?;
    if status.success() {
        Ok(bwrap)
    } else {
        Err(format!(
            "{} không tạo được namespace (kiểm tra user namespace / quyền hệ thống)",
            bwrap.display()
        ))
    }
}

/// Mô tả trạng thái để `tez doctor` và UI hiển thị.
pub fn status(policy: &SandboxPolicy) -> String {
    if !policy.enabled {
        return "tắt".to_string();
    }
    if !cfg!(target_os = "linux") {
        return "bật nhưng chưa hỗ trợ trên hệ điều hành này".to_string();
    }
    match probe() {
        Ok(path) => format!(
            "bật · {} · mạng {}",
            path.display(),
            if policy.allow_network {
                "cho phép"
            } else {
                "chặn"
            }
        ),
        Err(error) => {
            format!("BẬT nhưng không dùng được bubblewrap — {error}; run_command sẽ bị từ chối")
        }
    }
}

/// Tạo lệnh bubblewrap chạy `script` trong workspace.
pub fn build_command(
    policy: &SandboxPolicy,
    workspace: &Path,
    cwd: &Path,
    script: &str,
) -> Result<Command, String> {
    if !cfg!(target_os = "linux") {
        return Err("sandbox hiện chỉ hỗ trợ Linux".to_string());
    }
    if !policy.enabled {
        return Err("sandbox chưa bật".to_string());
    }
    let bwrap = bwrap_path().ok_or_else(|| {
        "không tìm thấy bubblewrap; cài `bubblewrap` hoặc tắt sandbox trong harness.json"
            .to_string()
    })?;
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("không mở được workspace: {error}"))?;
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("không mở được cwd: {error}"))?;
    if workspace == Path::new("/") {
        return Err("không sandbox được workspace gốc `/`".to_string());
    }
    let data_dir = policy
        .data_dir
        .canonicalize()
        .unwrap_or_else(|_| policy.data_dir.clone());
    if data_dir == Path::new("/") {
        return Err("data dir không hợp lệ: không thể sandbox khi data dir là `/`".to_string());
    }
    if workspace == data_dir {
        return Err(
            "workspace không được trùng data dir; chọn thư mục con như workspaces/default"
                .to_string(),
        );
    }

    let mut command = Command::new(&bwrap);
    command
        .arg("--die-with-parent")
        .arg("--unshare-all")
        // --unshare-all tách cả network namespace; chỉ nối lại khi người dùng
        // chủ động cho phép mạng.
        .args(["--ro-bind", "/", "/"])
        .args(["--dev", "/dev"])
        .args(["--proc", "/proc"])
        .args(["--tmpfs", "/tmp"])
        .args(["--bind"])
        .arg(&workspace)
        .arg(&workspace)
        .args(["--chdir"])
        .arg(&cwd);
    if policy.allow_network {
        command.arg("--share-net");
    }

    hide_data_dir(&mut command, &data_dir, &workspace);
    hide_home_secrets(&mut command, &workspace);

    // Không truyền toàn bộ env của app: API key/token có thể nằm trong env.
    command.env_clear();
    for key in SAFE_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("TMPDIR", "/tmp");

    command.arg("--").arg("/bin/sh").arg("-c").arg(script);
    Ok(command)
}

/// Tạo lệnh bubblewrap cho một chương trình MCP stdio.
///
/// Khác `run_command`, MCP server thường cần đọc code plugin trong data dir
/// (ví dụ `plugins/.../server.py`), nên chỉ ẩn các file/dữ liệu nhạy cảm thay
/// vì ẩn toàn bộ data dir.
pub fn build_mcp_command(
    policy: &SandboxPolicy,
    cwd: &Path,
    writable_paths: &[PathBuf],
    program: &str,
    args: &[String],
) -> Result<Command, String> {
    if !cfg!(target_os = "linux") {
        return Err("sandbox MCP hiện chỉ hỗ trợ Linux".to_string());
    }
    if !policy.enabled {
        return Err("sandbox MCP chưa bật".to_string());
    }
    let bwrap = bwrap_path().ok_or_else(|| {
        "không tìm thấy bubblewrap; cài `bubblewrap` hoặc tắt sandbox cho server MCP".to_string()
    })?;
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("không mở được cwd của MCP server: {error}"))?;
    if cwd == Path::new("/") {
        return Err("không sandbox được MCP server với cwd là `/`".to_string());
    }
    let data_dir = policy
        .data_dir
        .canonicalize()
        .unwrap_or_else(|_| policy.data_dir.clone());
    if data_dir == Path::new("/") {
        return Err("data dir không hợp lệ: không thể sandbox khi data dir là `/`".to_string());
    }
    if cwd == data_dir {
        return Err("cwd của MCP server không được trùng data dir".to_string());
    }

    let mut command = Command::new(&bwrap);
    command
        .arg("--die-with-parent")
        .arg("--unshare-all")
        .args(["--ro-bind", "/", "/"])
        .args(["--dev", "/dev"])
        .args(["--proc", "/proc"])
        .args(["--tmpfs", "/tmp"])
        .args(["--bind"])
        .arg(&cwd)
        .arg(&cwd)
        .args(["--chdir"])
        .arg(&cwd);

    for path in writable_paths {
        let path = path
            .canonicalize()
            .map_err(|error| format!("không mở được thư mục ghi {}: {error}", path.display()))?;
        if path == Path::new("/") {
            return Err("không được bind `/` read-write vào sandbox MCP".to_string());
        }
        if path == data_dir || path.starts_with(&data_dir) {
            return Err(format!(
                "không được bind data dir hoặc thư mục con của nó read-write: {}",
                path.display()
            ));
        }
        if data_dir.starts_with(&path) {
            return Err(format!(
                "không được bind thư mục cha của data dir read-write: {}",
                path.display()
            ));
        }
        command.arg("--bind").arg(&path).arg(&path);
    }

    if policy.allow_network {
        command.arg("--share-net");
    }

    hide_data_secrets(&mut command, &data_dir, &cwd);
    hide_home_secrets(&mut command, &cwd);

    command.env_clear();
    for key in SAFE_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("TMPDIR", "/tmp");

    // Dùng shell để resolve command theo PATH, sau đó exec thay thế shell.
    command
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg("exec \"$0\" \"$@\"")
        .arg(program)
        .args(args);
    Ok(command)
}

/// Các biến môi trường không chứa secret nhưng cần cho toolchain.
const SAFE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "PNPM_HOME",
    "JAVA_HOME",
    "GOPATH",
    "GOROOT",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "XDG_RUNTIME_DIR",
];

/// Ẩn data dir. Nếu workspace nằm trong data dir (workspace mặc định), chỉ ẩn
/// các mục anh em để không ẩn luôn workspace.
fn hide_data_dir(command: &mut Command, data_dir: &Path, workspace: &Path) {
    if data_dir == workspace {
        return;
    }
    if !workspace.starts_with(data_dir) {
        hide_path(command, data_dir);
        return;
    }
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if workspace.starts_with(&path) {
            continue;
        }
        hide_path(command, &path);
    }
    // Các workspace khác không được đọc từ workspace hiện tại.
    let workspaces = data_dir.join("workspaces");
    if let Ok(entries) = std::fs::read_dir(&workspaces) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path == workspace {
                continue;
            }
            hide_path(command, &path);
        }
    }
}

/// Ẩn các file/dữ liệu nhạy cảm trong data dir nhưng vẫn cho MCP server đọc
/// code plugin/skill. Dùng cho MCP stdio, nơi server có thể nằm trong
/// `plugins/` của app.
fn hide_data_secrets(command: &mut Command, data_dir: &Path, workspace: &Path) {
    for name in [
        "master.key",
        "providers.json",
        "providers.json.bak",
        "connection.json",
        "connection.json.bak",
        "mcp.json",
        "mcp.json.bak",
        "harness.json",
        "oauth.json",
        "approvals.json",
        "tez.db",
        "tez.db-wal",
        "tez.db-shm",
        "audit.jsonl",
        "audit.jsonl.1",
    ] {
        hide_path(command, &data_dir.join(name));
    }
    for name in [
        "sessions", "memory", "plans", "images", "logs", "backups", "updates",
    ] {
        let path = data_dir.join(name);
        if workspace.starts_with(&path) {
            continue;
        }
        hide_path(command, &path);
    }
}

/// Ẩn các file/thư mục credential phổ biến trong HOME.
fn hide_home_secrets(command: &mut Command, workspace: &Path) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    for relative in [
        ".ssh",
        ".gnupg",
        ".aws",
        ".azure",
        ".kube",
        ".docker",
        ".config/gh",
        ".config/gcloud",
        ".config/op",
        ".password-store",
        ".netrc",
        ".git-credentials",
        ".npmrc",
        ".pypirc",
        ".cargo/credentials.toml",
    ] {
        let path = home.join(relative);
        if workspace.starts_with(&path) {
            continue;
        }
        hide_path(command, &path);
    }
}

fn hide_path(command: &mut Command, path: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    let target = if metadata.file_type().is_symlink() {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    if target == Path::new("/") {
        return;
    }
    if target.is_dir() {
        command.arg("--tmpfs").arg(&target);
    } else {
        // /dev/null là file rỗng; bind đè lên file credential.
        command.args(["--ro-bind", "/dev/null"]).arg(&target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_enabled_only_on_linux() {
        let settings = SandboxSettings::default();
        assert_eq!(settings.enabled, cfg!(target_os = "linux"));
        assert!(!settings.allow_network);
    }

    #[test]
    fn status_mentions_missing_bwrap() {
        let policy = SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: PathBuf::from("/tmp/tez-data"),
        };
        let text = status(&policy);
        if cfg!(target_os = "linux") && bwrap_path().is_none() {
            assert!(text.contains("bubblewrap"));
            assert!(text.contains("từ chối"));
        } else {
            assert!(!text.is_empty());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn build_command_blocks_network_by_default() {
        if bwrap_path().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: dir.path().join("data"),
        };
        let command = build_command(&policy, &workspace, &workspace, "true").unwrap();
        let debug = format!("{command:?}");
        assert!(debug.contains("--unshare-all"));
        assert!(!debug.contains("--share-net"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_workspace_equal_to_data_dir() {
        if bwrap_path().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let policy = SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: data.clone(),
        };
        let error = build_command(&policy, &data, &data, "true").unwrap_err();
        assert!(error.contains("data dir"), "unexpected error: {error}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mcp_command_hides_secrets_and_blocks_network() {
        if bwrap_path().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let cwd = dir.path().join("workspace");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(data.join("master.key"), "secret").unwrap();
        let policy = SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: data.clone(),
        };
        let command = build_mcp_command(
            &policy,
            &cwd,
            std::slice::from_ref(&cache),
            "python3",
            &["server.py".to_string()],
        )
        .unwrap();
        let debug = format!("{command:?}");
        assert!(debug.contains("--unshare-all"));
        assert!(!debug.contains("--share-net"));
        assert!(debug.contains("master.key"));
        assert!(debug.contains(&cache.display().to_string()));
        assert!(command
            .as_std()
            .get_args()
            .any(|arg| arg == "exec \"$0\" \"$@\""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mcp_command_rejects_writable_data_dir() {
        if bwrap_path().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let policy = SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: data.clone(),
        };
        let error = build_mcp_command(&policy, &cwd, std::slice::from_ref(&data), "python3", &[])
            .unwrap_err();
        assert!(error.contains("read-write"), "unexpected error: {error}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mcp_command_rejects_writable_data_dir_parent() {
        if bwrap_path().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let policy = SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: data.clone(),
        };
        let error = build_mcp_command(
            &policy,
            &cwd,
            std::slice::from_ref(&dir.path().to_path_buf()),
            "python3",
            &[],
        )
        .unwrap_err();
        assert!(error.contains("thư mục cha"), "unexpected error: {error}");
    }
}
