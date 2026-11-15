//! Kiểm tra và tải bản cập nhật từ GitHub Releases.
//!
//! Không dùng plugin updater của Tauri để giữ luồng đơn giản, dễ hiểu và dùng
//! chung được cho cả CLI lẫn desktop. Việc cài đặt do từng nền tảng đảm nhận.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Repo mặc định; có thể ghi đè bằng biến môi trường `TEZ_GITHUB_REPO`.
pub const DEFAULT_REPO: &str = "teaching-ez/tez";
const USER_AGENT: &str = "tez-updater";

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

/// So sánh phiên bản kiểu semver đơn giản (đủ cho `0.1.0`, `1.2.3`, `v1.2.3-beta`).
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
    match (a_pre, b_pre) {
        (None, Some(_)) => true,
        (Some(_), None) => false,
        _ => false,
    }
}

fn parse_version(value: &str) -> (Vec<u64>, Option<String>) {
    let trimmed = value.trim().trim_start_matches(['v', 'V']);
    let (core, rest) = match trimmed.split_once('-') {
        Some((core, rest)) => (core, Some(rest)),
        None => (trimmed, None),
    };
    let core = core.split('+').next().unwrap_or(core);
    let numbers = core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();
    let pre = rest
        .map(|value| value.split('+').next().unwrap_or(value).to_string())
        .filter(|value| !value.is_empty());
    (numbers, pre)
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

/// Chọn asset phù hợp với hệ điều hành và kiến trúc hiện tại.
///
/// `platform` nhận dạng `"linux-x86_64"` (OS-ARCH). Asset ghi kiến trúc khác
/// bị loại, không còn fallback về asset đầu tiên.
pub fn pick_asset<'a>(info: &'a UpdateInfo, platform: &str) -> Option<&'a UpdateAsset> {
    let platform = platform.to_lowercase();
    let os_keywords: &[&str] = if platform.contains("windows") {
        &["windows", "win64", "win32", ".msi", ".exe"]
    } else if platform.contains("macos") || platform.contains("darwin") || platform.contains("osx")
    {
        &["macos", "darwin", "osx", ".dmg", ".app.tar.gz"]
    } else {
        &["linux", "appimage", ".deb", ".rpm", ".tar.gz"]
    };
    const ARM64: &[&str] = &["aarch64", "arm64"];
    const X86_64: &[&str] = &["x86_64", "amd64", "x64"];
    const X86: &[&str] = &["i686", "i386", "x86"];

    let arch_keywords: Option<&[&str]> = if platform.contains("aarch64")
        || platform.contains("arm64")
    {
        Some(ARM64)
    } else if platform.contains("x86_64") || platform.contains("amd64") || platform.contains("x64")
    {
        Some(X86_64)
    } else if platform.contains("i686") || platform.contains("i386") {
        Some(X86)
    } else {
        // Caller chỉ truyền OS: dùng kiến trúc của binary đang chạy.
        match std::env::consts::ARCH {
            "aarch64" => Some(ARM64),
            "x86_64" => Some(X86_64),
            "x86" | "i686" => Some(X86),
            _ => None,
        }
    };

    const ANY_ARCH: &[&str] = &[
        "x86_64",
        "amd64",
        "x64",
        "aarch64",
        "arm64",
        "i686",
        "i386",
        "armv7",
        "universal",
    ];

    let mut best: Option<(u8, &UpdateAsset)> = None;
    for asset in &info.assets {
        let name = asset.name.to_lowercase();
        if [".sha256", ".sha512", ".sig", ".asc", ".txt", ".json"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            continue;
        }
        if !os_keywords.iter().any(|keyword| name.contains(keyword)) {
            continue;
        }
        let score = match arch_keywords {
            Some(keywords) if keywords.iter().any(|keyword| name.contains(keyword)) => 2,
            Some(_) if ANY_ARCH.iter().any(|keyword| name.contains(keyword)) => continue,
            _ => 1,
        };
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, asset));
        }
    }
    best.map(|(_, asset)| asset)
}

/// Tải asset về `dest_dir`, trả về đường dẫn file đã lưu.
pub async fn download(
    asset: &UpdateAsset,
    dest_dir: impl AsRef<Path>,
) -> Result<PathBuf, UpdateError> {
    let dest_dir = dest_dir.as_ref();
    std::fs::create_dir_all(dest_dir).map_err(|error| UpdateError::Io(error.to_string()))?;

    let file_name = asset
        .name
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("tez-update");
    let path = dest_dir.join(file_name);

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|error| UpdateError::Http(error.to_string()))?;

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
    let bytes = response
        .bytes()
        .await
        .map_err(|error| UpdateError::Http(error.to_string()))?;
    std::fs::write(&path, &bytes).map_err(|error| UpdateError::Io(error.to_string()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn picks_platform_asset() {
        let info = UpdateInfo {
            current: "0.1.0".into(),
            latest: "0.2.0".into(),
            available: true,
            notes: String::new(),
            url: String::new(),
            published_at: None,
            assets: vec![
                UpdateAsset {
                    name: "tez_0.2.0_amd64.deb".into(),
                    url: "u1".into(),
                    size: 1,
                },
                UpdateAsset {
                    name: "tez_0.2.0_x64-setup.exe".into(),
                    url: "u2".into(),
                    size: 2,
                },
            ],
        };
        assert_eq!(pick_asset(&info, "windows").unwrap().url, "u2");
        assert_eq!(pick_asset(&info, "linux").unwrap().url, "u1");
    }

    #[test]
    fn picks_asset_for_requested_architecture() {
        let info = UpdateInfo {
            current: "0.2.0".into(),
            latest: "0.3.0".into(),
            available: true,
            notes: String::new(),
            url: String::new(),
            published_at: None,
            assets: vec![
                UpdateAsset {
                    name: "tez_0.3.0_linux_aarch64.tar.gz".into(),
                    url: "arm".into(),
                    size: 1,
                },
                UpdateAsset {
                    name: "tez_0.3.0_linux_x86_64.tar.gz".into(),
                    url: "x86".into(),
                    size: 1,
                },
                UpdateAsset {
                    name: "tez_0.3.0_macos_aarch64.dmg".into(),
                    url: "mac".into(),
                    size: 1,
                },
                UpdateAsset {
                    name: "checksums.txt".into(),
                    url: "sum".into(),
                    size: 1,
                },
            ],
        };
        assert_eq!(pick_asset(&info, "linux-x86_64").unwrap().url, "x86");
        assert_eq!(pick_asset(&info, "linux-aarch64").unwrap().url, "arm");
        assert_eq!(pick_asset(&info, "macos-aarch64").unwrap().url, "mac");
        assert!(pick_asset(&info, "windows-x86_64").is_none());
    }
}
