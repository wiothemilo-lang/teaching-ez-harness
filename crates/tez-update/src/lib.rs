//! Kiểm tra và tải bản cập nhật từ GitHub Releases.
//!
//! Không dùng plugin updater của Tauri để giữ luồng đơn giản, dễ hiểu và dùng
//! chung được cho cả CLI lẫn desktop. Việc cài đặt do từng nền tảng đảm nhận.

use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Repo mặc định; có thể ghi đè bằng biến môi trường `TEZ_GITHUB_REPO`.
pub const DEFAULT_REPO: &str = "teaching-ez/tez";
const USER_AGENT: &str = "tez-updater";
/// Giới hạn cứng cho một asset cập nhật (512 MB).
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
/// Checksum chỉ là văn bản nhỏ; giới hạn riêng để tránh tải nhầm file lớn.
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("không gọi được GitHub: {0}")]
    Http(String),
    #[error("repo '{0}' chưa có bản phát hành nào (hoặc không công khai)")]
    NotFound(String),
    #[error("GitHub trả lỗi: {0}")]
    Api(String),
    #[error("không đọc được phản hồi: {0}")]
    Parse(String),
    #[error("không ghi được file: {0}")]
    Io(String),
    #[error("file tải về quá lớn: {size} byte (giới hạn {limit} byte)")]
    TooLarge { size: u64, limit: u64 },
    #[error("checksum SHA-256 không khớp cho '{asset}': mong đợi {expected}, nhận {actual}")]
    Checksum {
        asset: String,
        expected: String,
        actual: String,
    },
    #[error("bản phát hành không có checksum SHA-256 cho '{0}'")]
    MissingChecksum(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub notes: String,
    pub url: String,
    pub published_at: Option<String>,
    pub assets: Vec<UpdateAsset>,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Debug, Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn repo() -> String {
    std::env::var("TEZ_GITHUB_REPO")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_string())
}

/// So sánh phiên bản theo semver: phần core, prerelease và bỏ build metadata.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, a_pre) = parse_version(latest);
    let (b, b_pre) = parse_version(current);
    let len = a.len().max(b.len());
    for index in 0..len {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    match (a_pre.as_deref(), b_pre.as_deref()) {
        (None, None) => false,
        // Bản stable luôn mới hơn prerelease cùng core version.
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => compare_prerelease(a, b) == Ordering::Greater,
    }
}

fn parse_version(value: &str) -> (Vec<u64>, Option<String>) {
    let trimmed = value.trim().trim_start_matches(['v', 'V']);
    // Build metadata sau dấu '+' không tham gia so sánh, và có thể chứa '-'.
    let without_build = trimmed.split('+').next().unwrap_or(trimmed);
    let (core, rest) = match without_build.split_once('-') {
        Some((core, rest)) => (core, Some(rest)),
        None => (without_build, None),
    };
    let numbers = core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();
    let pre = rest.filter(|value| !value.is_empty()).map(str::to_string);
    (numbers, pre)
}

fn compare_prerelease(left: &str, right: &str) -> Ordering {
    let mut left = left.split('.');
    let mut right = right.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left), Some(right)) => {
                let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    // Identifier số thấp hơn identifier chữ theo semver.
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => left.cmp(right),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

/// Kiểm tra bản mới nhất của repo mặc định.
pub async fn check(current: &str) -> Result<UpdateInfo, UpdateError> {
    check_repo(current, &repo()).await
}

/// Kiểm tra bản mới nhất của một repo GitHub (`owner/name`).
pub async fn check_repo(current: &str, repo: &str) -> Result<UpdateInfo, UpdateError> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| UpdateError::Http(error.to_string()))?;

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| UpdateError::Http(error.to_string()))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(UpdateError::NotFound(repo.to_string()));
    }
    if !response.status().is_success() {
        return Err(UpdateError::Api(format!(
            "{} khi gọi {}",
            response.status(),
            url
        )));
    }

    let release: Release = response
        .json()
        .await
        .map_err(|error| UpdateError::Parse(error.to_string()))?;

    let latest = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    Ok(UpdateInfo {
        current: current.to_string(),
        available: is_newer(&latest, current),
        latest,
        notes: release.body.unwrap_or_default(),
        url: release
            .html_url
            .unwrap_or_else(|| format!("https://github.com/{repo}/releases/latest")),
        published_at: release.published_at,
        assets: release
            .assets
            .into_iter()
            .map(|asset| UpdateAsset {
                name: asset.name,
                url: asset.browser_download_url,
                size: asset.size,
            })
            .collect(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Os {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arch {
    Arm64,
    X86_64,
    X86,
    Armv7,
}

enum ArchSpec {
    /// Không ghi kiến trúc; dùng kiến trúc của binary đang chạy.
    Current,
    Known(Arch),
    /// Có ghi kiến trúc nhưng không nhận ra (ví dụ riscv64).
    Unknown,
}

/// Tách tên asset / platform thành các token theo mọi ký tự không phải chữ-số.
///
/// Nhờ vậy `x86` không còn khớp nhầm với `x86_64`: tên `..._x86_64...` tách
/// thành `["x86", "64"]` và được nhận diện bằng cặp token riêng.
fn tokens(value: &str) -> Vec<&str> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect()
}

fn has_token(tokens: &[&str], needle: &str) -> bool {
    tokens.contains(&needle)
}

fn has_x86_64(tokens: &[&str]) -> bool {
    tokens
        .windows(2)
        .any(|pair| pair[0] == "x86" && pair[1] == "64")
}

fn arch_from_tokens(tokens: &[&str]) -> Option<Arch> {
    if has_token(tokens, "aarch64") || has_token(tokens, "arm64") {
        Some(Arch::Arm64)
    } else if has_x86_64(tokens) || has_token(tokens, "amd64") || has_token(tokens, "x64") {
        Some(Arch::X86_64)
    } else if has_token(tokens, "i686") || has_token(tokens, "i386") || has_token(tokens, "x86") {
        Some(Arch::X86)
    } else if has_token(tokens, "armv7") {
        Some(Arch::Armv7)
    } else {
        None
    }
}

fn parse_os(platform: &str) -> Option<Os> {
    let platform = platform.to_lowercase();
    let tokens = tokens(&platform);
    if has_token(&tokens, "windows")
        || has_token(&tokens, "win")
        || has_token(&tokens, "win64")
        || has_token(&tokens, "win32")
    {
        Some(Os::Windows)
    } else if has_token(&tokens, "macos")
        || has_token(&tokens, "darwin")
        || has_token(&tokens, "osx")
        || has_token(&tokens, "mac")
    {
        Some(Os::Macos)
    } else if has_token(&tokens, "linux") {
        Some(Os::Linux)
    } else {
        None
    }
}

fn parse_arch(platform: &str) -> ArchSpec {
    let platform = platform.to_lowercase();
    let tokens = tokens(&platform);
    if let Some(arch) = arch_from_tokens(&tokens) {
        return ArchSpec::Known(arch);
    }

    const PLATFORM_WORDS: &[&str] = &[
        "windows",
        "win",
        "win64",
        "win32",
        "macos",
        "darwin",
        "osx",
        "mac",
        "linux",
        "gnu",
        "musl",
        "msvc",
        "unknown",
        "pc",
        "apple",
        "universal",
    ];
    if tokens.iter().any(|token| !PLATFORM_WORDS.contains(token)) {
        ArchSpec::Unknown
    } else {
        ArchSpec::Current
    }
}

fn current_arch() -> Option<Arch> {
    match std::env::consts::ARCH {
        "aarch64" => Some(Arch::Arm64),
        "x86_64" => Some(Arch::X86_64),
        "x86" | "i686" => Some(Arch::X86),
        "arm" | "armv7" => Some(Arch::Armv7),
        _ => None,
    }
}

fn matches_os(tokens: &[&str], name: &str, os: Os) -> bool {
    match os {
        Os::Windows => {
            has_token(tokens, "windows")
                || has_token(tokens, "win")
                || has_token(tokens, "win64")
                || has_token(tokens, "win32")
                || name.ends_with(".exe")
                || name.ends_with(".msi")
        }
        Os::Macos => {
            has_token(tokens, "macos")
                || has_token(tokens, "darwin")
                || has_token(tokens, "osx")
                || has_token(tokens, "mac")
                || name.ends_with(".dmg")
                || name.ends_with(".app.tar.gz")
        }
        Os::Linux => {
            // `.tar.gz` cũng xuất hiện trong tên gói macOS; đừng chọn nhầm.
            if has_token(tokens, "macos")
                || has_token(tokens, "darwin")
                || has_token(tokens, "osx")
                || has_token(tokens, "mac")
                || name.ends_with(".app.tar.gz")
            {
                return false;
            }
            has_token(tokens, "linux")
                || has_token(tokens, "appimage")
                || name.ends_with(".deb")
                || name.ends_with(".rpm")
                || name.ends_with(".tar.gz")
        }
    }
}

fn has_arch_marker(tokens: &[&str]) -> bool {
    has_x86_64(tokens)
        || [
            "aarch64",
            "arm64",
            "amd64",
            "x64",
            "i686",
            "i386",
            "x86",
            "armv7",
            "universal",
            "ppc64",
            "s390x",
            "riscv64",
        ]
        .iter()
        .any(|marker| has_token(tokens, marker))
}

fn installer_score(os: Os, name: &str, tokens: &[&str]) -> u8 {
    if os != Os::Windows {
        return 0;
    }
    if name.ends_with(".msi") {
        3
    } else if name.ends_with(".exe")
        && (has_token(tokens, "setup")
            || has_token(tokens, "nsis")
            || has_token(tokens, "installer"))
    {
        2
    } else if name.ends_with(".exe") {
        // `tez.exe` trần không phải installer; chỉ dùng khi không còn lựa chọn.
        0
    } else {
        1
    }
}

/// Chọn asset phù hợp với hệ điều hành và kiến trúc hiện tại.
///
/// `platform` nhận dạng `"linux-x86_64"` (OS-ARCH). Asset ghi kiến trúc khác
/// bị loại; platform không nhận ra trả về `None` thay vì đoán Linux.
pub fn pick_asset<'a>(info: &'a UpdateInfo, platform: &str) -> Option<&'a UpdateAsset> {
    let os = parse_os(platform)?;
    let wanted = match parse_arch(platform) {
        ArchSpec::Known(arch) => arch,
        ArchSpec::Current => current_arch()?,
        ArchSpec::Unknown => return None,
    };

    let mut best: Option<(u16, &UpdateAsset)> = None;
    for asset in &info.assets {
        let name = asset.name.to_lowercase();
        if [
            ".sha256",
            ".sha256sum",
            ".sha512",
            ".sig",
            ".asc",
            ".txt",
            ".json",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        {
            continue;
        }
        let tokens = tokens(&name);
        if !matches_os(&tokens, &name, os) {
            continue;
        }

        let is_universal = has_token(&tokens, "universal");
        let arch_score: u16 = match arch_from_tokens(&tokens) {
            Some(arch) if arch == wanted => 2,
            Some(_) => continue,
            None if is_universal => {
                if os == Os::Macos {
                    1
                } else {
                    continue;
                }
            }
            None if has_arch_marker(&tokens) => continue,
            None => 1,
        };
        // Ưu tiên installer trên Windows, sau đó mới tới kiến trúc khớp.
        let score = u16::from(installer_score(os, &name, &tokens)) * 10 + arch_score;
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, asset));
        }
    }
    best.map(|(_, asset)| asset)
}

fn basename(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value)
}

fn is_checksum_name(name: &str) -> bool {
    let name = name.to_lowercase();
    name.ends_with(".sha256") || name.ends_with(".sha256sum") || name.ends_with(".txt")
}

fn checksum_candidates<'a>(
    assets: &'a [UpdateAsset],
    target: &UpdateAsset,
) -> Vec<&'a UpdateAsset> {
    let target_name = basename(&target.name).to_lowercase();
    let mut specific = Vec::new();
    let mut generic = Vec::new();
    for asset in assets {
        if asset.url == target.url || asset.name.eq_ignore_ascii_case(&target.name) {
            continue;
        }
        if !is_checksum_name(&asset.name) {
            continue;
        }
        if !target_name.is_empty() && asset.name.to_lowercase().contains(&target_name) {
            specific.push(asset);
        } else {
            generic.push(asset);
        }
    }
    specific.extend(generic);
    specific
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Tìm hash SHA-256 của `target_name` trong nội dung một file checksum.
///
/// Hỗ trợ định dạng GNU (`<hash>  <file>`), BSD (`SHA256 (<file>) = <hash>`),
/// `<file>: <hash>`, và file chỉ chứa một hash khi tên file checksum gắn với
/// đúng asset (`allow_bare`).
fn parse_checksum(text: &str, target_name: &str, allow_bare: bool) -> Option<String> {
    let target = basename(target_name).to_lowercase();
    let mut bare = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let upper = line.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("SHA256 (") {
            if let Some((file, hash)) = rest.split_once(") = ") {
                if basename(file).eq_ignore_ascii_case(&target) && is_sha256_hex(hash.trim()) {
                    return Some(hash.trim().to_lowercase());
                }
            }
        }

        if let Some((file, hash)) = line.rsplit_once(':') {
            let hash = hash.trim();
            if is_sha256_hex(hash) && basename(file.trim()).eq_ignore_ascii_case(&target) {
                return Some(hash.to_lowercase());
            }
        }

        let mut parts = line.split_whitespace();
        let Some(first) = parts.next() else {
            continue;
        };
        if !is_sha256_hex(first) {
            continue;
        }
        let rest = line[first.len()..].trim();
        if rest.is_empty() {
            if allow_bare && bare.is_none() {
                bare = Some(first.to_lowercase());
            }
            continue;
        }
        let file = rest.trim_start_matches('*').trim();
        if basename(file).eq_ignore_ascii_case(&target) {
            return Some(first.to_lowercase());
        }
    }
    bare
}

async fn fetch_checksum_text(
    client: &reqwest::Client,
    asset: &UpdateAsset,
) -> Result<String, UpdateError> {
    if asset.size > MAX_CHECKSUM_BYTES {
        return Err(UpdateError::TooLarge {
            size: asset.size,
            limit: MAX_CHECKSUM_BYTES,
        });
    }
    let response = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|error| UpdateError::Http(error.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::Api(format!(
            "{} khi tải checksum {}",
            response.status(),
            asset.url
        )));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_CHECKSUM_BYTES {
            return Err(UpdateError::TooLarge {
                size: length,
                limit: MAX_CHECKSUM_BYTES,
            });
        }
    }

    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| UpdateError::Http(error.to_string()))?;
        let next = data.len() as u64 + chunk.len() as u64;
        if next > MAX_CHECKSUM_BYTES {
            return Err(UpdateError::TooLarge {
                size: next,
                limit: MAX_CHECKSUM_BYTES,
            });
        }
        data.extend_from_slice(&chunk);
    }
    String::from_utf8(data)
        .map_err(|error| UpdateError::Parse(format!("checksum không phải UTF-8: {error}")))
}

async fn fetch_expected_checksum(
    client: &reqwest::Client,
    assets: &[UpdateAsset],
    target: &UpdateAsset,
) -> Result<String, UpdateError> {
    let candidates = checksum_candidates(assets, target);
    if candidates.is_empty() {
        return Err(UpdateError::MissingChecksum(target.name.clone()));
    }

    let target_name = basename(&target.name).to_lowercase();
    let mut fetched_any = false;
    let mut last_error = None;
    for candidate in candidates {
        let text = match fetch_checksum_text(client, candidate).await {
            Ok(text) => {
                fetched_any = true;
                text
            }
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let allow_bare =
            !target_name.is_empty() && candidate.name.to_lowercase().contains(&target_name);
        if let Some(expected) = parse_checksum(&text, &target.name, allow_bare) {
            return Ok(expected);
        }
    }

    if !fetched_any {
        if let Some(error) = last_error {
            return Err(error);
        }
    }
    Err(UpdateError::MissingChecksum(target.name.clone()))
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn verify_digest(actual: &str, expected: &str, asset: &str) -> Result<(), UpdateError> {
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(UpdateError::Checksum {
            asset: asset.to_string(),
            expected: expected.trim().to_lowercase(),
            actual: actual.to_lowercase(),
        })
    }
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize().as_slice())
}

#[cfg(test)]
fn verify_sha256(bytes: &[u8], expected: &str, asset: &str) -> Result<(), UpdateError> {
    verify_digest(&sha256_hex(bytes), expected, asset)
}

/// File tạm trong cùng thư mục đích; tự xoá nếu chưa được commit.
struct TempFile {
    path: PathBuf,
    committed: bool,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn temp_path(dest_dir: &Path, file_name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    dest_dir.join(format!(".{file_name}.{}.{nonce}.part", std::process::id()))
}

fn set_update_permissions(path: &Path, file_name: &str) -> Result<(), UpdateError> {
    #[cfg(unix)]
    if file_name.to_ascii_lowercase().ends_with(".appimage") {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(path, permissions)
            .map_err(|error| UpdateError::Io(error.to_string()))?;
    }
    #[cfg(not(unix))]
    let _ = (path, file_name);
    Ok(())
}

/// Tải asset về `dest_dir`, xác minh SHA-256 rồi mới đặt tên chính thức.
///
/// `assets` là toàn bộ asset của cùng bản phát hành, dùng để tìm file checksum
/// (`.sha256`, `.sha256sum`, hoặc `.txt`). Không có checksum thì từ chối tải.
pub async fn download(
    asset: &UpdateAsset,
    assets: &[UpdateAsset],
    dest_dir: impl AsRef<Path>,
) -> Result<PathBuf, UpdateError> {
    if asset.size > MAX_DOWNLOAD_BYTES {
        return Err(UpdateError::TooLarge {
            size: asset.size,
            limit: MAX_DOWNLOAD_BYTES,
        });
    }

    let file_name = basename(&asset.name);
    let file_name = if file_name.is_empty() || file_name == "." || file_name == ".." {
        "tez-update"
    } else {
        file_name
    };
    let dest_dir = dest_dir.as_ref();
    std::fs::create_dir_all(dest_dir).map_err(|error| UpdateError::Io(error.to_string()))?;
    let path = dest_dir.join(file_name);

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|error| UpdateError::Http(error.to_string()))?;

    let expected = fetch_expected_checksum(&client, assets, asset).await?;

    let response = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|error| UpdateError::Http(error.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::Api(format!(
            "{} khi tải {}",
            response.status(),
            asset.url
        )));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_DOWNLOAD_BYTES {
            return Err(UpdateError::TooLarge {
                size: length,
                limit: MAX_DOWNLOAD_BYTES,
            });
        }
    }

    let mut temp = TempFile::new(temp_path(dest_dir, file_name));
    let mut file =
        std::fs::File::create(&temp.path).map_err(|error| UpdateError::Io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| UpdateError::Http(error.to_string()))?;
        total = total.saturating_add(chunk.len() as u64);
        if total > MAX_DOWNLOAD_BYTES {
            return Err(UpdateError::TooLarge {
                size: total,
                limit: MAX_DOWNLOAD_BYTES,
            });
        }
        file.write_all(&chunk)
            .map_err(|error| UpdateError::Io(error.to_string()))?;
        hasher.update(&chunk);
    }
    file.flush()
        .map_err(|error| UpdateError::Io(error.to_string()))?;
    drop(file);

    let actual = hex_digest(hasher.finalize().as_slice());
    verify_digest(&actual, &expected, &asset.name)?;
    set_update_permissions(&temp.path, file_name)?;
    std::fs::rename(&temp.path, &path).map_err(|error| {
        UpdateError::Io(format!("không thay thế được {}: {error}", path.display()))
    })?;
    temp.commit();
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::*;

    fn asset(name: &str, url: &str) -> UpdateAsset {
        UpdateAsset {
            name: name.into(),
            url: url.into(),
            size: 1,
        }
    }

    fn release_info(assets: Vec<UpdateAsset>) -> UpdateInfo {
        UpdateInfo {
            current: "0.1.0".into(),
            latest: "0.2.0".into(),
            available: true,
            notes: String::new(),
            url: String::new(),
            published_at: None,
            assets,
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "tez-update-test-{}-{label}-{id}",
            std::process::id()
        ))
    }

    /// Server HTTP tối giản: trả lần lượt các body, mỗi request một kết nối.
    fn spawn_server(bodies: Vec<Vec<u8>>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let handle = thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().expect("accept test request");
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).expect("write header");
                stream.write_all(&body).expect("write body");
            }
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "0.1.0-beta"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn compares_prerelease_identifiers() {
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(is_newer("0.2.0-beta.10", "0.2.0-beta.2"));
        assert!(is_newer("0.2.0-beta.1", "0.2.0-beta"));
        assert!(is_newer("0.2.0-rc.1", "0.2.0-beta.11"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0-beta.1"));
        assert!(is_newer("0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer("0.2.0-rc.1", "0.2.0"));
        // Build metadata bị bỏ qua hoàn toàn.
        assert!(!is_newer("0.2.0+build.5", "0.2.0"));
        assert!(!is_newer("0.2.0-beta.1+build.9", "0.2.0-beta.1+build.1"));
        assert!(is_newer("0.2.0-beta.2+build.1", "0.2.0-beta.1+build.9"));
    }

    #[test]
    fn picks_platform_asset() {
        let info = release_info(vec![
            asset("tez_0.2.0_amd64.deb", "u1"),
            asset("tez_0.2.0_x64-setup.exe", "u2"),
        ]);
        assert_eq!(pick_asset(&info, "windows").unwrap().url, "u2");
        assert_eq!(pick_asset(&info, "linux").unwrap().url, "u1");
    }

    #[test]
    fn picks_asset_for_requested_architecture() {
        let info = release_info(vec![
            asset("tez_0.3.0_linux_aarch64.tar.gz", "arm"),
            asset("tez_0.3.0_linux_x86_64.tar.gz", "x86"),
            asset("tez_0.3.0_macos_aarch64.dmg", "mac"),
            asset("checksums.txt", "sum"),
        ]);
        assert_eq!(pick_asset(&info, "linux-x86_64").unwrap().url, "x86");
        assert_eq!(pick_asset(&info, "linux-aarch64").unwrap().url, "arm");
        assert_eq!(pick_asset(&info, "macos-aarch64").unwrap().url, "mac");
        assert!(pick_asset(&info, "windows-x86_64").is_none());
    }

    #[test]
    fn distinguishes_i686_from_x86_64() {
        let info = release_info(vec![
            asset("tez_0.3.0_linux_i686.deb", "i686"),
            asset("tez_0.3.0_linux_x86_64.deb", "x86_64"),
        ]);
        assert_eq!(pick_asset(&info, "linux-i686").unwrap().url, "i686");
        assert_eq!(pick_asset(&info, "linux-x86_64").unwrap().url, "x86_64");

        // Token `x86` đơn lẻ không được khớp với `x86_64`.
        let only_x86 = release_info(vec![asset("tez_0.3.0_linux_x86.deb", "x86")]);
        assert!(pick_asset(&only_x86, "linux-x86_64").is_none());
        assert_eq!(pick_asset(&only_x86, "linux-i686").unwrap().url, "x86");
    }

    #[test]
    fn picks_universal_macos_asset_for_any_arch() {
        let info = release_info(vec![asset("tez_0.3.0_macos_universal.dmg", "universal")]);
        assert_eq!(pick_asset(&info, "macos-aarch64").unwrap().url, "universal");
        assert_eq!(pick_asset(&info, "macos-x86_64").unwrap().url, "universal");
        assert_eq!(
            pick_asset(&info, "macos-universal").unwrap().url,
            "universal"
        );
    }

    #[test]
    fn does_not_use_macos_tarball_for_linux() {
        let info = release_info(vec![asset("tez_0.3.0_macos_aarch64.app.tar.gz", "mac")]);
        assert!(pick_asset(&info, "linux-aarch64").is_none());
        assert_eq!(pick_asset(&info, "macos-aarch64").unwrap().url, "mac");
    }

    #[test]
    fn prefers_windows_installers_over_bare_exe() {
        let info = release_info(vec![
            asset("tez.exe", "bare"),
            asset("tez_0.2.0_x64-setup.exe", "nsis"),
            asset("tez_0.2.0_x64_en-US.msi", "msi"),
        ]);
        assert_eq!(pick_asset(&info, "windows-x86_64").unwrap().url, "msi");

        let without_msi = release_info(vec![
            asset("tez.exe", "bare"),
            asset("tez_0.2.0_x64-setup.exe", "nsis"),
        ]);
        assert_eq!(
            pick_asset(&without_msi, "windows-x86_64").unwrap().url,
            "nsis"
        );
    }

    #[test]
    fn rejects_unknown_platforms() {
        let info = release_info(vec![asset("tez_0.3.0_linux_x86_64.tar.gz", "linux")]);
        assert!(pick_asset(&info, "freebsd-x86_64").is_none());
        assert!(pick_asset(&info, "linux-riscv64").is_none());
    }

    #[test]
    fn parses_checksum_lines() {
        let target = "tez_0.2.0_linux_x86_64.tar.gz";
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        let text = format!("{second}  tez_0.2.0_linux_aarch64.tar.gz\n{first} *{target}\n");
        assert_eq!(parse_checksum(&text, target, false), Some(first.clone()));
        assert_eq!(parse_checksum(&text, "missing.tar.gz", false), None);

        let bsd = format!("SHA256 ({target}) = {first}\n");
        assert_eq!(parse_checksum(&bsd, target, false), Some(first));

        // Hash trần chỉ hợp lệ khi file checksum gắn với đúng asset.
        let bare = "c".repeat(64);
        assert_eq!(parse_checksum(&bare, target, true), Some(bare.clone()));
        assert_eq!(parse_checksum(&bare, target, false), None);
    }

    #[test]
    fn prefers_checksum_asset_named_after_target() {
        let target = asset("tez_0.2.0_linux_x86_64.tar.gz", "target");
        let assets = vec![
            asset("checksums.txt", "generic"),
            asset("tez_0.2.0_linux_x86_64.tar.gz.sha256", "specific"),
        ];
        let candidates = checksum_candidates(&assets, &target);
        assert_eq!(candidates[0].url, "specific");
        assert_eq!(candidates[1].url, "generic");
    }

    #[test]
    fn verifies_checksum_match_and_mismatch() {
        let bytes = b"tez update payload";
        let expected = sha256_hex(bytes);
        assert!(verify_sha256(bytes, &expected, "tez.AppImage").is_ok());
        assert!(verify_sha256(bytes, &expected.to_uppercase(), "tez.AppImage").is_ok());

        let error = verify_sha256(bytes, &"0".repeat(64), "tez.AppImage").unwrap_err();
        assert!(matches!(error, UpdateError::Checksum { .. }));
    }

    #[tokio::test]
    async fn downloads_and_verifies_checksum() {
        let bytes = b"verified payload".to_vec();
        let checksum = format!("{}  tez.AppImage\n", sha256_hex(&bytes));
        let (base, server) = spawn_server(vec![checksum.into_bytes(), bytes.clone()]);
        let dir = temp_dir("verified");

        let target = UpdateAsset {
            name: "tez.AppImage".into(),
            url: format!("{base}/tez.AppImage"),
            size: bytes.len() as u64,
        };
        let checksum_asset = UpdateAsset {
            name: "tez.AppImage.sha256".into(),
            url: format!("{base}/tez.AppImage.sha256"),
            size: 1,
        };
        let path = download(&target, &[checksum_asset, target.clone()], &dir)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rejects_checksum_mismatch() {
        let bytes = b"tampered payload".to_vec();
        let checksum = format!("{}  tez.AppImage\n", "0".repeat(64));
        let (base, server) = spawn_server(vec![checksum.into_bytes(), bytes.clone()]);
        let dir = temp_dir("mismatch");

        let target = UpdateAsset {
            name: "tez.AppImage".into(),
            url: format!("{base}/tez.AppImage"),
            size: bytes.len() as u64,
        };
        let checksum_asset = UpdateAsset {
            name: "tez.AppImage.sha256".into(),
            url: format!("{base}/tez.AppImage.sha256"),
            size: 1,
        };
        let error = download(&target, &[checksum_asset, target.clone()], &dir)
            .await
            .unwrap_err();

        assert!(matches!(error, UpdateError::Checksum { .. }));
        assert!(!dir.join("tez.AppImage").exists());
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn refuses_download_without_checksum_asset() {
        let target = UpdateAsset {
            name: "tez.AppImage".into(),
            url: "https://example.invalid/tez.AppImage".into(),
            size: 10,
        };
        let dir = temp_dir("no-checksum");
        let error = download(&target, &[], &dir).await.unwrap_err();
        assert!(matches!(error, UpdateError::MissingChecksum(_)));
        assert!(!dir.join("tez.AppImage").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rejects_assets_over_size_limit() {
        let oversized = UpdateAsset {
            name: "tez.AppImage".into(),
            url: "https://example.invalid/tez.AppImage".into(),
            size: MAX_DOWNLOAD_BYTES + 1,
        };
        let dir = temp_dir("oversized");
        let error = download(&oversized, &[], &dir).await.unwrap_err();
        assert!(matches!(error, UpdateError::TooLarge { .. }));
        assert!(!dir.exists());
    }
}
