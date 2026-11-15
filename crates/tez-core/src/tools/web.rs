//! Tool `web_fetch`: tải và đọc nội dung HTTP(S) trong giới hạn an toàn.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;
use tokio::io::AsyncReadExt;

use super::{Tool, ToolError, ToolOutput};

const DEFAULT_MAX_CHARS: usize = 20_000;
const MIN_MAX_CHARS: u64 = 1_000;
const MAX_MAX_CHARS: u64 = 200_000;
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
const MAX_ERROR_PREVIEW_CHARS: usize = 500;
/// `--dump-dom` có thể trả về DOM rất lớn; chặn trước khi đưa vào bộ nhớ.
const MAX_BROWSER_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const USER_AGENT: &str = "tez/0.1 (+local)";
const MAX_REDIRECTS: usize = 5;

/// Tải một URL http/https và trả về text đã làm sạch.
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "Tải một URL http/https và trả về text đã làm sạch. Chỉ dùng cho trang công khai; URL nội bộ, localhost và địa chỉ IP riêng bị chặn."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL http/https cần tải" },
                    "max_chars": {
                        "type": "integer",
                        "minimum": MIN_MAX_CHARS,
                        "maximum": MAX_MAX_CHARS,
                        "description": "Số ký tự text tối đa trả về (mặc định 20000)"
                    },
                    "render": {
                        "type": "string",
                        "enum": ["static", "browser"],
                        "description": "static (mặc định) tải HTML; browser dùng Chromium headless nếu có để chạy JavaScript"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'url'".into()))?;
        let max_chars = match args.get("max_chars") {
            None | Some(Value::Null) => DEFAULT_MAX_CHARS,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| ToolError::InvalidArgs("max_chars phải là số nguyên".into()))?
                .clamp(MIN_MAX_CHARS, MAX_MAX_CHARS) as usize,
        };
        let render = args
            .get("render")
            .and_then(Value::as_str)
            .unwrap_or("static");
        match render {
            "static" => fetch_url(url, max_chars, false).await,
            "browser" => fetch_with_browser(url, max_chars).await,
            other => Err(ToolError::InvalidArgs(format!(
                "render phải là 'static' hoặc 'browser', nhận '{other}'"
            ))),
        }
    }
}

/// Tải URL và dựng output. `allow_private` chỉ dùng trong test để gọi server
/// cục bộ; mọi đường chạy thật đều truyền `false`.
async fn fetch_url(
    raw_url: &str,
    max_chars: usize,
    allow_private: bool,
) -> Result<ToolOutput, ToolError> {
    let (mut url, mut pin) = validate_url(raw_url, allow_private).await?;
    let mut response = None;

    for _ in 0..=MAX_REDIRECTS {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT);
        if let Some((host, addresses)) = &pin {
            builder = builder.resolve_to_addrs(host, addresses);
        }
        let client = builder.build().map_err(|error| {
            ToolError::Execution(format!("không tạo được HTTP client: {error}"))
        })?;

        let current = match client.get(url.clone()).send().await {
            Ok(response) => response,
            Err(error) => return Ok(ToolOutput::error(format!("không tải được URL: {error}"))),
        };

        if current.status().is_redirection() {
            let Some(location) = current
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
            else {
                return Ok(ToolOutput::error(
                    "redirect nhưng thiếu header Location".to_string(),
                ));
            };
            let next = match url.join(location) {
                Ok(next) => next,
                Err(error) => {
                    return Ok(ToolOutput::error(format!("redirect không hợp lệ: {error}")))
                }
            };
            let (next, next_pin) = match validate_url(next.as_str(), allow_private).await {
                Ok(value) => value,
                Err(error) => {
                    return Ok(ToolOutput::error(format!(
                        "redirect tới địa chỉ bị chặn: {error}"
                    )))
                }
            };
            url = next;
            pin = next_pin;
            continue;
        }

        response = Some(current);
        break;
    }

    let Some(mut response) = response else {
        return Ok(ToolOutput::error(
            "quá nhiều redirect (tối đa 5)".to_string(),
        ));
    };
    let status = response.status();
    let final_url = response.url().to_string();

    if !status.is_success() {
        let body = match read_body_capped(&mut response, MAX_ERROR_BODY_BYTES).await {
            Ok((body, _)) => body,
            Err(error) => {
                return Ok(ToolOutput::error(format!(
                    "HTTP {status} (không đọc được body: {error})"
                )))
            }
        };
        let body = String::from_utf8_lossy(&body);
        let preview: String = body.chars().take(MAX_ERROR_PREVIEW_CHARS).collect();
        return Ok(ToolOutput::error(format!("HTTP {status}\n\n{preview}")));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let lower_content_type = content_type
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if lower_content_type.starts_with("application/pdf")
        || lower_content_type.starts_with("audio/")
        || lower_content_type.starts_with("video/")
    {
        return Ok(ToolOutput::error(format!(
            "chưa hỗ trợ nội dung '{lower_content_type}': web_fetch chỉ đọc HTML/text. \
             Hãy tải file trực tiếp nếu cần."
        )));
    }
    let is_html = lower_content_type.contains("html");

    let (body, body_truncated) = match read_body_capped(&mut response, MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => return Ok(ToolOutput::error(format!("không đọc được body: {error}"))),
    };
    let body = decode_body(&body, content_type.as_deref());
    let text = if is_html { strip_html(&body) } else { body };
    let (text, truncated) = truncate_chars(&text, max_chars);

    let mut content = format!("URL: {final_url}\nStatus: {}\n\n{text}", status.as_u16());
    if body_truncated {
        content.push_str("\n… (body đã cắt ở 512 KB)");
    }
    if truncated {
        content.push_str("\n… (đã cắt bớt)");
    }
    Ok(ToolOutput::ok(content))
}

/// Kiểm tra scheme, userinfo và đích đến trước khi gửi request.
async fn fetch_with_browser(raw_url: &str, max_chars: usize) -> Result<ToolOutput, ToolError> {
    let Some(binary) = browser_binary() else {
        return fallback_to_static(raw_url, max_chars, "không tìm thấy Chromium/Chrome").await;
    };
    let (_url, pin) = match validate_url(raw_url, false).await {
        Ok(value) => value,
        Err(error) => return fallback_to_static(raw_url, max_chars, &error.to_string()).await,
    };
    let output = match pin {
        Some((host, addresses)) => {
            run_browser(binary, raw_url, Some(&host), &addresses, max_chars).await?
        }
        None => run_browser(binary, raw_url, None, &[], max_chars).await?,
    };
    if !output.is_error {
        Ok(output)
    } else {
        fallback_to_static(raw_url, max_chars, &output.content).await
    }
}

/// Browser không chạy được thì vẫn trả nội dung tĩnh, kèm lý do để người dùng biết.
async fn fallback_to_static(
    raw_url: &str,
    max_chars: usize,
    reason: &str,
) -> Result<ToolOutput, ToolError> {
    let mut output = fetch_url(raw_url, max_chars, false).await?;
    let reason: String = reason.chars().take(300).collect();
    output.content = format!("[browser fallback: {reason}]\n\n{}", output.content);
    Ok(output)
}

/// Chạy browser với binary đã chọn. `host`/`addresses` là kết quả phân giải đã
/// kiểm tra, dùng để pin DNS qua `--host-resolver-rules`.
async fn run_browser(
    binary: &Path,
    url: &str,
    host: Option<&str>,
    addresses: &[SocketAddr],
    max_chars: usize,
) -> Result<ToolOutput, ToolError> {
    let profile = std::env::temp_dir().join(format!(
        "tez-browser-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default()
    ));
    let resolver = browser_resolver_rules(host, addresses);
    let mut command = tokio::process::Command::new(binary);
    command.arg("--headless=new").arg("--disable-gpu");
    if browser_no_sandbox() {
        command.arg("--no-sandbox");
    }
    command
        .arg("--disable-dev-shm-usage")
        .arg("--virtual-time-budget=5000")
        .arg(format!("--host-resolver-rules={resolver}"))
        .arg("--user-data-dir")
        .arg(&profile)
        .arg("--dump-dom")
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&profile);
            return Ok(ToolOutput::error(format!(
                "không chạy được browser: {error}"
            )));
        }
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = std::fs::remove_dir_all(&profile);
        return Ok(ToolOutput::error(
            "không mở được stdout của browser".to_string(),
        ));
    };

    let mut html = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(25), async {
        // Đọc tối đa cap + 1 byte để biết output có vượt trần hay không.
        let mut limited = (&mut stdout).take((MAX_BROWSER_OUTPUT_BYTES + 1) as u64);
        limited.read_to_end(&mut html).await
    })
    .await;
    let _ = child.start_kill();
    let status = child.wait().await;
    let _ = std::fs::remove_dir_all(&profile);

    match read {
        Err(_) => return Ok(ToolOutput::error("browser quá thời gian (25s)".to_string())),
        Ok(Err(error)) => {
            return Ok(ToolOutput::error(format!(
                "không đọc được output browser: {error}"
            )))
        }
        Ok(Ok(_)) => {}
    }
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return Ok(ToolOutput::error(format!(
                "không lấy được trạng thái browser: {error}"
            )))
        }
    };
    let output_truncated = html.len() > MAX_BROWSER_OUTPUT_BYTES;
    // Khi output vượt trần, chính ta đã kill browser nên status có thể là signal.
    if !output_truncated && !status.success() {
        return Ok(ToolOutput::error(format!("browser thoát với lỗi {status}")));
    }

    if output_truncated {
        html.truncate(MAX_BROWSER_OUTPUT_BYTES);
    }
    let html = String::from_utf8_lossy(&html);
    let text = strip_html(&html);
    let (text, truncated) = truncate_chars(&text, max_chars);
    let mut content = format!("URL: {url}\nStatus: browser\n\n{text}");
    if output_truncated {
        content.push_str("\n… (DOM đã cắt ở 2 MB)");
    }
    if truncated {
        content.push_str("\n\n… (đã cắt bớt)");
    }
    Ok(ToolOutput::ok(content))
}

fn browser_resolver_rules(host: Option<&str>, addresses: &[SocketAddr]) -> String {
    match host {
        Some(host) if !addresses.is_empty() => {
            let mut rules = vec!["MAP * ~NOTFOUND".to_string(), format!("EXCLUDE {host}")];
            for address in addresses {
                rules.push(format!("MAP {host} {address}"));
            }
            rules.join(", ")
        }
        _ => "MAP * ~NOTFOUND".to_string(),
    }
}

fn browser_no_sandbox() -> bool {
    std::env::var("TEZ_BROWSER_NO_SANDBOX")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

fn browser_binary() -> Option<&'static PathBuf> {
    static BROWSER: OnceLock<Option<PathBuf>> = OnceLock::new();
    BROWSER
        .get_or_init(|| {
            if let Some(path) = std::env::var_os("TEZ_CHROMIUM") {
                let path = PathBuf::from(path);
                if path.is_file() {
                    return Some(path);
                }
            }
            for name in [
                "chromium",
                "chromium-browser",
                "google-chrome",
                "google-chrome-stable",
                "chrome",
                "msedge",
            ] {
                if let Some(path) = find_in_path(name) {
                    return Some(path);
                }
            }
            None
        })
        .as_ref()
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

async fn validate_url(
    raw: &str,
    allow_private: bool,
) -> Result<(reqwest::Url, Option<(String, Vec<SocketAddr>)>), ToolError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ToolError::InvalidArgs("URL rỗng".into()));
    }
    let mut url = reqwest::Url::parse(raw)
        .map_err(|error| ToolError::InvalidArgs(format!("URL không hợp lệ: {error}")))?;

    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ToolError::InvalidArgs(format!(
                "chỉ hỗ trợ http/https, không hỗ trợ '{other}'"
            )))
        }
    }

    if has_userinfo(raw) || !url.username().is_empty() || url.password().is_some() {
        return Err(ToolError::InvalidArgs(
            "URL không được chứa thông tin đăng nhập (userinfo)".into(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| ToolError::InvalidArgs("URL thiếu host".into()))?
        .to_string();
    // `host_str` trả IPv6 kèm dấu ngoặc vuông; bỏ dấu cuối để so khớp.
    // Dấu chấm cuối (FQDN) phải bị cắt ở cả URL gửi đi và khóa pin DNS, nếu
    // không `resolve_to_addrs` sẽ không khớp host và reqwest tự phân giải lại.
    let trimmed_host = host.trim_end_matches('.');
    if trimmed_host.is_empty() {
        return Err(ToolError::InvalidArgs("URL thiếu host".into()));
    }
    if trimmed_host != host.as_str() {
        url.set_host(Some(trimmed_host)).map_err(|error| {
            ToolError::InvalidArgs(format!("host không hợp lệ sau khi chuẩn hóa: {error}"))
        })?;
    }
    let host = trimmed_host.to_ascii_lowercase();
    let ip_candidate = host.trim_start_matches('[').trim_end_matches(']');

    if let Ok(ip) = ip_candidate.parse::<IpAddr>() {
        if !allow_private && is_forbidden_ip(ip) {
            return Err(ToolError::InvalidArgs(format!(
                "từ chối địa chỉ IP nội bộ: {ip}"
            )));
        }
        return Ok((url, None));
    }

    if !allow_private && is_localhost_name(&host) {
        return Err(ToolError::InvalidArgs(format!(
            "từ chối host nội bộ: {host}"
        )));
    }
    if allow_private {
        return Ok((url, None));
    }

    let port = url.port_or_known_default().unwrap_or(80);
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|error| {
            ToolError::Execution(format!("không phân giải được host '{host}': {error}"))
        })?;
    let mut addresses_out = Vec::new();
    for address in addresses {
        if is_forbidden_ip(address.ip()) {
            return Err(ToolError::InvalidArgs(format!(
                "host '{host}' trỏ tới địa chỉ nội bộ ({})",
                address.ip()
            )));
        }
        addresses_out.push(address);
    }
    if addresses_out.is_empty() {
        return Err(ToolError::Execution(format!(
            "không phân giải được host '{host}'"
        )));
    }
    Ok((url, Some((host, addresses_out))))
}

/// Kiểm tra hình dạng URL không cần DNS; dùng trong test redirect.
#[cfg(test)]
fn forbidden_url_shape(url: &reqwest::Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Some(format!("scheme '{}' không được phép", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Some("URL chứa userinfo".to_string());
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Some("URL thiếu host".to_string());
    }
    let ip_candidate = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = ip_candidate.parse::<IpAddr>() {
        if is_forbidden_ip(ip) {
            return Some(format!("địa chỉ IP nội bộ: {ip}"));
        }
        return None;
    }
    if is_localhost_name(&host) {
        return Some(format!("host nội bộ: {host}"));
    }
    None
}

/// URL có phần userinfo (`scheme://user:pass@host`) hay không.
fn has_userinfo(raw: &str) -> bool {
    let Some((_, rest)) = raw.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@')
}

fn is_localhost_name(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost")
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_forbidden_ipv4(ip),
        IpAddr::V6(ip) => is_forbidden_ipv6(ip),
    }
}

fn is_forbidden_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        // CGNAT 100.64.0.0/10.
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        // "This network" 0.0.0.0/8.
        || octets[0] == 0
        // IETF protocol assignments 192.0.0.0/24.
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        // Benchmarking 198.18.0.0/15.
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))
        // Reserved 240.0.0.0/4.
        || octets[0] >= 240
}

fn is_forbidden_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    // IPv4-mapped/compatible (::ffff:127.0.0.1, ::127.0.0.1).
    if let Some(mapped) = ip.to_ipv4() {
        if is_forbidden_ipv4(mapped) {
            return true;
        }
    }
    let segments = ip.segments();
    let first = segments[0];
    // fc00::/7 (unique local).
    if first & 0xfe00 == 0xfc00 {
        return true;
    }
    // fe80::/10 (link-local).
    if first & 0xffc0 == 0xfe80 {
        return true;
    }
    // 2001:db8::/32 (documentation).
    if first == 0x2001 && segments[1] == 0x0db8 {
        return true;
    }
    // Teredo 2001::/32 (IPv6 tunnel qua IPv4).
    if first == 0x2001 && segments[1] == 0 {
        return true;
    }
    // NAT64 64:ff9b::/96: IPv4 nhúng có thể trỏ vào mạng nội bộ.
    if first == 0x0064
        && segments[1] == 0xff9b
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0
    {
        return true;
    }
    // NAT64 local-use 64:ff9b:1::/48 (RFC 8215).
    if first == 0x0064 && segments[1] == 0xff9b && segments[2] == 1 {
        return true;
    }
    // 6to4 2002::/16: kiểm tra IPv4 nhúng.
    if first == 0x2002 {
        let embedded = Ipv4Addr::new(
            (segments[1] >> 8) as u8,
            segments[1] as u8,
            (segments[2] >> 8) as u8,
            segments[2] as u8,
        );
        return is_forbidden_ipv4(embedded);
    }
    false
}

/// Đọc body theo chunk, dừng khi đạt `cap` byte.
async fn read_body_capped(
    response: &mut reqwest::Response,
    cap: usize,
) -> Result<(Vec<u8>, bool), reqwest::Error> {
    let mut body = Vec::new();
    let mut truncated = false;
    while body.len() < cap {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        if chunk.is_empty() {
            break;
        }
        let room = cap - body.len();
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    // Body đúng bằng cap vẫn có thể còn dữ liệu phía sau.
    if !truncated && body.len() == cap {
        if let Some(chunk) = response.chunk().await? {
            if !chunk.is_empty() {
                truncated = true;
            }
        }
    }
    Ok((body, truncated))
}

/// Tìm `>` kết thúc thẻ, bỏ qua `>` nằm trong giá trị thuộc tính có nháy.
fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, ch) in input[start..].char_indices() {
        match quote {
            Some(expected) => {
                if ch == expected {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '>' => return Some(start + offset),
                _ => {}
            },
        }
    }
    None
}

/// Thẻ mở có tự đóng bằng `/>` hay không. Trong HTML, `/` dính liền sau giá trị
/// thuộc tính không nháy (`src=foo/`) là một phần giá trị, không phải tự đóng.
fn is_self_closing(tag_text: &str) -> bool {
    let Some(before_slash) = tag_text.trim_end().strip_suffix('/') else {
        return false;
    };
    match before_slash.chars().last() {
        Some('"' | '\'') => true,
        Some(ch) if ch.is_whitespace() => true,
        _ => {
            let tag_name_end = before_slash
                .find(|ch: char| ch.is_ascii_whitespace() || ch == '/')
                .unwrap_or(before_slash.len());
            before_slash.len() == tag_name_end
        }
    }
}

/// Bỏ hoàn toàn comment HTML, kể cả khi bên trong có dấu `>`.
fn remove_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        match after.find("-->") {
            Some(end) => rest = &after[end + 3..],
            // Comment không đóng: bỏ phần còn lại để tránh lộ nội dung.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Bỏ `<script>...</script>` và `<style>...</style>` (không phân biệt hoa thường).
/// Comment được giữ nguyên và bỏ qua khi tìm thẻ, để `<script>` nằm trong
/// comment không bị coi là thẻ thật.
fn remove_tag_blocks(input: &str, tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while cursor < input.len() {
        let open_relative = lower[cursor..].find(&open);
        let comment_relative = lower[cursor..].find("<!--");
        if let Some(comment_relative) = comment_relative {
            if open_relative.is_none_or(|open_relative| comment_relative < open_relative) {
                let comment_start = cursor + comment_relative;
                let Some(end_relative) = lower[comment_start + 4..].find("-->") else {
                    // Comment không đóng: giữ phần còn lại cho `remove_comments` xử lý.
                    out.push_str(&input[cursor..]);
                    return out;
                };
                let comment_end = comment_start + 4 + end_relative + 3;
                out.push_str(&input[cursor..comment_end]);
                cursor = comment_end;
                continue;
            }
        }
        let Some(relative) = open_relative else {
            break;
        };
        let start = cursor + relative;
        let after = lower[start + open.len()..].chars().next();
        let is_boundary =
            after.is_none_or(|ch| ch.is_ascii_whitespace() || matches!(ch, '>' | '/'));
        if !is_boundary {
            out.push_str(&input[cursor..start + open.len()]);
            cursor = start + open.len();
            continue;
        }

        let Some(open_end) = find_tag_end(&lower, start) else {
            // Không có `>` kết thúc thẻ mở: bỏ phần còn lại cho an toàn.
            out.push_str(&input[cursor..start]);
            cursor = input.len();
            break;
        };
        let self_closing = is_self_closing(&lower[start..open_end]);

        let Some(close_relative) = lower[open_end..].find(&close) else {
            if self_closing {
                // `<script src="x"/>` không có nội dung: chỉ bỏ thẻ, giữ phần
                // sau. Nhưng nếu ngay sau thẻ là text (không phải thẻ khác),
                // coi đó là script chưa đóng để không lộ nội dung.
                let next = input[open_end + 1..].chars().find(|ch| !ch.is_whitespace());
                if next.is_none_or(|ch| ch == '<') {
                    out.push_str(&input[cursor..start]);
                    out.push(' ');
                    cursor = open_end + 1;
                    continue;
                }
            }
            // Không có thẻ đóng: bỏ phần còn lại để tránh lộ nội dung script/style.
            out.push_str(&input[cursor..start]);
            cursor = input.len();
            break;
        };
        let close_start = open_end + close_relative;
        let Some(gt_relative) = lower[close_start..].find('>') else {
            out.push_str(&input[cursor..start]);
            cursor = input.len();
            break;
        };
        out.push_str(&input[cursor..start]);
        out.push(' ');
        cursor = close_start + gt_relative + 1;
    }

    out.push_str(&input[cursor..]);
    out
}

/// Thay thẻ HTML bằng khoảng trắng, giữ nguyên dấu `<` không phải thẻ.
fn replace_tags_with_space(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            let is_tag = chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?'));
            if is_tag {
                for next in chars.by_ref() {
                    if next == '>' {
                        break;
                    }
                }
                out.push(' ');
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Giải mã entity số (`&#123;`, `&#x1F;`), trả về ký tự và số byte đã dùng.
fn numeric_entity(tail: &str) -> Option<(char, usize)> {
    let body = tail.strip_prefix("&#")?;
    let (digits, radix, prefix) = match body.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16, 1),
        None => (body, 10, 0),
    };
    let end = digits.find(';')?;
    if end == 0 || end > 8 {
        return None;
    }
    let value = u32::from_str_radix(&digits[..end], radix).ok()?;
    Some((char::from_u32(value)?, 2 + prefix + end + 1))
}

/// Decode một lượt các entity phổ biến, tránh giải mã lặp (`&amp;lt;`).
fn decode_entities(input: &str) -> String {
    const ENTITIES: &[(&str, char)] = &[
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
        ("&#39;", '\''),
        ("&apos;", '\''),
        ("&nbsp;", ' '),
        ("&copy;", '©'),
        ("&reg;", '®'),
        ("&trade;", '™'),
        ("&mdash;", '—'),
        ("&ndash;", '–'),
        ("&hellip;", '…'),
        ("&laquo;", '«'),
        ("&raquo;", '»'),
        ("&lsquo;", '‘'),
        ("&rsquo;", '’'),
        ("&ldquo;", '“'),
        ("&rdquo;", '”'),
        ("&euro;", '€'),
        ("&pound;", '£'),
        ("&deg;", '°'),
        ("&middot;", '·'),
        ("&bull;", '•'),
    ];

    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        let tail = &rest[position..];
        if let Some((value, consumed)) = numeric_entity(tail) {
            out.push(value);
            rest = &tail[consumed..];
            continue;
        }
        let mut matched = false;
        for (entity, value) in ENTITIES {
            if let Some(stripped) = tail.strip_prefix(entity) {
                out.push(*value);
                rest = stripped;
                matched = true;
                break;
            }
        }
        if !matched {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Decode body theo charset trong Content-Type; mặc định UTF-8 lossy.
fn decode_body(bytes: &[u8], content_type: Option<&str>) -> String {
    let charset = content_type.and_then(|value| {
        value.split(';').find_map(|part| {
            let part = part.trim();
            part.strip_prefix("charset=")
                .map(|value| value.trim_matches('"').to_ascii_lowercase())
        })
    });
    match charset.as_deref() {
        Some("iso-8859-1" | "latin1" | "latin-1" | "windows-1252" | "cp1252") => {
            bytes.iter().map(|byte| cp1252_char(*byte)).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Windows-1252 cho 0x80–0x9F; phần còn lại trùng Unicode Latin-1.
fn cp1252_char(byte: u8) -> char {
    match byte {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        0x81 | 0x8D | 0x8F | 0x90 | 0x9D => '\u{FFFD}',
        _ => byte as char,
    }
}

/// Gộp khoảng trắng trong từng dòng và bỏ dòng trống.
fn collapse_whitespace(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut normalized = String::with_capacity(line.len());
        let mut last_space = false;
        for ch in line.chars() {
            if ch.is_whitespace() {
                if !last_space && !normalized.is_empty() {
                    normalized.push(' ');
                }
                last_space = true;
            } else {
                normalized.push(ch);
                last_space = false;
            }
        }
        while normalized.ends_with(' ') {
            normalized.pop();
        }
        if !normalized.is_empty() {
            lines.push(normalized);
        }
    }
    lines.join("\n")
}

fn strip_html(html: &str) -> String {
    // Bỏ script/style trước (bỏ qua comment), rồi mới bỏ comment. Nhờ vậy dấu
    // `<!--` nằm trong script không nuốt phần còn lại của trang.
    let without_scripts = remove_tag_blocks(html, "script");
    let without_styles = remove_tag_blocks(&without_scripts, "style");
    let without_comments = remove_comments(&without_styles);
    let text = replace_tags_with_space(&without_comments);
    let decoded = decode_entities(&text);
    collapse_whitespace(&decoded)
}

/// Cắt theo số ký tự (không theo byte) để không tách giữa ký tự UTF-8.
fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => (text[..index].to_string(), true),
        None => (text.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let result = validate_url("file:///etc/passwd", false).await;
        assert!(result.is_err());
    }

    #[test]
    fn redirect_shape_blocks_private_targets() {
        for raw in [
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://localhost/",
            "file:///etc/passwd",
            "http://user:pass@example.com/",
        ] {
            let url = reqwest::Url::parse(raw).unwrap();
            assert!(forbidden_url_shape(&url).is_some(), "{raw} phải bị chặn");
        }
        let public = reqwest::Url::parse("https://example.com/path").unwrap();
        assert!(forbidden_url_shape(&public).is_none());
    }

    #[tokio::test]
    async fn rejects_local_and_private_hosts() {
        let urls = [
            "http://localhost/",
            "http://localhost:8080/",
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://[::1]/",
        ];
        for url in urls {
            let result = validate_url(url, false).await;
            assert!(result.is_err(), "{url} phải bị chặn");
        }
    }

    #[tokio::test]
    async fn rejects_userinfo() {
        let result = validate_url("http://user:pass@example.com/", false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn trims_trailing_dot_from_request_url_and_pin_key() {
        let (url, pin) = validate_url("http://Example.COM./path", true)
            .await
            .unwrap();
        assert_eq!(url.host_str(), Some("example.com"));
        assert_eq!(url.as_str(), "http://example.com/path");
        assert!(pin.is_none());
    }

    #[tokio::test]
    async fn rejects_trailing_dot_localhost() {
        let result = validate_url("http://localhost./", false).await;
        assert!(result.is_err());
    }

    #[test]
    fn strip_html_removes_scripts_styles_and_decodes_entities() {
        let html = "<html><head><style>body { color: red; }</style></head>\
                    <body><h1>Xin chào</h1><script>alert('x')</script>\
                    <p>A &amp; B &lt; C&nbsp;D &#39;ok&#39; &quot;q&quot;</p></body></html>";
        let text = strip_html(html);
        assert!(!text.contains("alert"), "{text}");
        assert!(!text.contains("color: red"), "{text}");
        assert!(text.contains("Xin chào"), "{text}");
        assert!(text.contains("A & B < C D 'ok' \"q\""), "{text}");
        assert!(!text.contains("\n\n"), "{text}");
    }

    #[test]
    fn strips_comments_even_when_they_contain_gt() {
        let html = "<p>before</p><!-- note > hidden <script>bad()</script> --><p>after</p>";
        let text = strip_html(html);
        assert!(text.contains("before"), "{text}");
        assert!(text.contains("after"), "{text}");
        assert!(!text.contains("hidden"), "{text}");
        assert!(!text.contains("bad()"), "{text}");
        assert!(!text.contains("-->"), "{text}");
    }

    #[test]
    fn script_comment_marker_does_not_drop_following_content() {
        let html = "<script>var marker = \"<!--\";</script><p>After script</p>";
        let text = strip_html(html);
        assert!(text.contains("After script"), "{text}");
        assert!(!text.contains("marker"), "{text}");
    }

    #[test]
    fn self_closing_script_and_style_keep_following_content() {
        let html = "<body><script src=\"app.js\"/><p>After script</p>\
                    <style media=\"screen\"/><p>After style</p></body>";
        let text = strip_html(html);
        assert!(text.contains("After script"), "{text}");
        assert!(text.contains("After style"), "{text}");
    }

    #[test]
    fn self_closing_script_does_not_leak_content() {
        for html in [
            "<p>before</p><script/>alert('secret')</script><p>after</p>",
            "<p>before</p><script src=\"app.js\"/>alert('secret')</script><p>after</p>",
            "<p>before</p><script/>alert('secret')",
        ] {
            let text = strip_html(html);
            assert!(text.contains("before"), "{text}");
            assert!(!text.contains("secret"), "{text}");
            assert!(!text.contains("alert"), "{text}");
        }
    }

    #[test]
    fn unclosed_script_and_style_drop_remainder() {
        for html in [
            "<p>Visible</p><script>alert('secret')",
            "<p>Visible</p><style>.secret { color: red; }",
        ] {
            let text = strip_html(html);
            assert!(text.contains("Visible"), "{text}");
            assert!(!text.contains("secret"), "{text}");
        }
    }

    #[test]
    fn decodes_common_named_entities() {
        let input = "&copy; &reg; &trade; &mdash; &ndash; &hellip; &laquo; &raquo; \
                     &lsquo; &rsquo; &ldquo; &rdquo; &euro; &pound; &deg; &middot; &bull;";
        assert_eq!(decode_entities(input), "© ® ™ — – … « » ‘ ’ “ ” € £ ° · •");
    }

    #[test]
    fn truncates_on_utf8_boundary() {
        let text = "á".repeat(10);
        let (cut, truncated) = truncate_chars(&text, 5);
        assert_eq!(cut, "ááááá");
        assert!(truncated);

        let (full, truncated) = truncate_chars("ngắn", 10);
        assert_eq!(full, "ngắn");
        assert!(!truncated);
    }

    #[test]
    fn blocks_special_ip_ranges() {
        for ip in [
            "100.64.0.1",
            "0.0.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::1",
            "2001::1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b:1::1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(is_forbidden_ip(ip), "{ip} phải bị chặn");
        }
        for ip in ["8.8.8.8", "2001:4860:4860::8888"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!is_forbidden_ip(ip), "{ip} không được chặn");
        }
    }

    #[test]
    fn decodes_numeric_entities() {
        assert_eq!(decode_entities("A &#65; &#x42; &#x1F600;"), "A A B 😀");
        assert_eq!(decode_entities("&#99999999;"), "&#99999999;");
    }

    #[test]
    fn decodes_windows_1252_charset() {
        let bytes = b"caf\xe9 \x93quote\x94";
        let text = decode_body(bytes, Some("text/plain; charset=windows-1252"));
        assert_eq!(text, "café “quote”");
        let text = decode_body(bytes, Some("text/plain; charset=utf-8"));
        assert!(text.contains("caf"));
    }

    #[tokio::test]
    async fn follows_redirect_and_revalidates() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
                "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string(),
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nFinal"
                    .to_string(),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let _ = socket.read(&mut request).await;
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });

        let url = format!("http://{address}/start");
        let output = fetch_url(&url, 20_000, true).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("Final"), "{}", output.content);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn fetches_html_end_to_end() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let body = "<html><body><h1>Hello</h1><script>bad()</script>\
                        <p>World &amp; friends</p></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });

        let url = format!("http://{address}/page");
        let output = fetch_url(&url, 20_000, true).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(
            output
                .content
                .starts_with(&format!("URL: {url}\nStatus: 200\n\n")),
            "{}",
            output.content
        );
        assert!(output.content.contains("Hello"), "{}", output.content);
        assert!(
            output.content.contains("World & friends"),
            "{}",
            output.content
        );
        assert!(!output.content.contains("bad()"), "{}", output.content);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn returns_error_for_non_2xx() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let body = "không tìm thấy";
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });

        let url = format!("http://{address}/missing");
        let output = fetch_url(&url, 20_000, true).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("404"), "{}", output.content);
        assert!(
            output.content.contains("không tìm thấy"),
            "{}",
            output.content
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn caps_output_to_max_chars() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let body = "a".repeat(2_000);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });

        let url = format!("http://{address}/long");
        let output = fetch_url(&url, 1_000, true).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(
            output.content.ends_with("… (đã cắt bớt)"),
            "{}",
            output.content
        );
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn browser_fallback_uses_injected_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("fake-chromium");
        std::fs::write(
            &binary,
            "#!/bin/sh\nprintf '<html><body>Rendered page</body></html>'\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

        let output = run_browser(&binary, "https://example.com/", None, &[], 20_000)
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(
            output.content.contains("Rendered page"),
            "{}",
            output.content
        );
        assert!(
            output.content.contains("Status: browser"),
            "{}",
            output.content
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn caps_browser_dump_dom_output() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-browser");
        std::fs::write(
            &script,
            "#!/bin/sh\nhead -c 3145728 /dev/zero | tr '\\0' 'x'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let output = run_browser(&script, "https://example.com/", None, &[], 20_000)
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(
            output.content.contains("DOM đã cắt ở 2 MB"),
            "{}",
            output.content
        );
        assert!(output.content.len() < 30_000, "{}", output.content.len());
    }

    #[test]
    fn browser_resolver_rules_pin_all_addresses() {
        let addresses = [
            SocketAddr::from(([93, 184, 216, 34], 443)),
            SocketAddr::from(([93, 184, 216, 35], 443)),
        ];
        let rules = browser_resolver_rules(Some("example.com"), &addresses);
        assert!(rules.contains("MAP * ~NOTFOUND"));
        assert!(rules.contains("EXCLUDE example.com"));
        assert!(rules.contains("MAP example.com 93.184.216.34:443"));
        assert!(rules.contains("MAP example.com 93.184.216.35:443"));
    }

    #[test]
    fn browser_resolver_rules_without_host_blocks_all() {
        assert_eq!(browser_resolver_rules(None, &[]), "MAP * ~NOTFOUND");
    }
}
