//! Tool `web_fetch`: tải và đọc nội dung HTTP(S) trong giới hạn an toàn.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

const DEFAULT_MAX_CHARS: usize = 20_000;
const MIN_MAX_CHARS: u64 = 1_000;
const MAX_MAX_CHARS: u64 = 200_000;
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
const MAX_ERROR_PREVIEW_CHARS: usize = 500;
const USER_AGENT: &str = "tez/0.1 (+local)";

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
        fetch_url(url, max_chars, false).await
    }
}

/// Tải URL và dựng output. `allow_private` chỉ dùng trong test để gọi server
/// cục bộ; mọi đường chạy thật đều truyền `false`.
async fn fetch_url(
    raw_url: &str,
    max_chars: usize,
    allow_private: bool,
) -> Result<ToolOutput, ToolError> {
    let url = validate_url(raw_url, allow_private).await?;
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("quá nhiều redirect");
        }
        if !allow_private && forbidden_url_shape(attempt.url()).is_some() {
            return attempt.error("redirect tới địa chỉ bị chặn");
        }
        attempt.follow()
    });
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .redirect(redirect_policy)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| ToolError::Execution(format!("không tạo được HTTP client: {error}")))?;

    let mut response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => return Ok(ToolOutput::error(format!("không tải được URL: {error}"))),
    };
    let status = response.status();
    let final_url = response.url().to_string();

    if !status.is_success() {
        let body = match read_body_capped(&mut response, MAX_ERROR_BODY_BYTES).await {
            Ok(body) => body,
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

    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("html"))
        .unwrap_or(false);

    let body = match read_body_capped(&mut response, MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => return Ok(ToolOutput::error(format!("không đọc được body: {error}"))),
    };
    let body = String::from_utf8_lossy(&body);
    let text = if is_html {
        strip_html(&body)
    } else {
        body.into_owned()
    };
    let (text, truncated) = truncate_chars(&text, max_chars);

    let mut content = format!("URL: {final_url}\nStatus: {}\n\n{text}", status.as_u16());
    if truncated {
        content.push_str("\n… (đã cắt bớt)");
    }
    Ok(ToolOutput::ok(content))
}

/// Kiểm tra scheme, userinfo và đích đến trước khi gửi request.
async fn validate_url(raw: &str, allow_private: bool) -> Result<reqwest::Url, ToolError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ToolError::InvalidArgs("URL rỗng".into()));
    }
    let url = reqwest::Url::parse(raw)
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
        .ok_or_else(|| ToolError::InvalidArgs("URL thiếu host".into()))?;
    // `host_str` trả IPv6 kèm dấu ngoặc vuông; bỏ dấu cuối để so khớp.
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err(ToolError::InvalidArgs("URL thiếu host".into()));
    }
    let ip_candidate = host.trim_start_matches('[').trim_end_matches(']');

    if let Ok(ip) = ip_candidate.parse::<IpAddr>() {
        if !allow_private && is_forbidden_ip(ip) {
            return Err(ToolError::InvalidArgs(format!(
                "từ chối địa chỉ IP nội bộ: {ip}"
            )));
        }
        return Ok(url);
    }

    if !allow_private && is_localhost_name(&host) {
        return Err(ToolError::InvalidArgs(format!(
            "từ chối host nội bộ: {host}"
        )));
    }
    if allow_private {
        return Ok(url);
    }

    let port = url.port_or_known_default().unwrap_or(80);
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|error| {
            ToolError::Execution(format!("không phân giải được host '{host}': {error}"))
        })?;
    let mut resolved = false;
    for address in addresses {
        resolved = true;
        if is_forbidden_ip(address.ip()) {
            return Err(ToolError::InvalidArgs(format!(
                "host '{host}' trỏ tới địa chỉ nội bộ ({})",
                address.ip()
            )));
        }
    }
    if !resolved {
        return Err(ToolError::Execution(format!(
            "không phân giải được host '{host}'"
        )));
    }
    Ok(url)
}

/// Kiểm tra hình dạng URL không cần DNS, dùng cho từng redirect.
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
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
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
    let first = ip.segments()[0];
    // fc00::/7 (unique local).
    if first & 0xfe00 == 0xfc00 {
        return true;
    }
    // fe80::/10 (link-local).
    first & 0xffc0 == 0xfe80
}

/// Đọc body theo chunk, dừng khi đạt `cap` byte.
async fn read_body_capped(
    response: &mut reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::new();
    while body.len() < cap {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        if chunk.is_empty() {
            break;
        }
        let room = cap - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if chunk.len() >= room {
            break;
        }
    }
    Ok(body)
}

/// Bỏ `<script>...</script>` và `<style>...</style>` (không phân biệt hoa thường).
fn remove_tag_blocks(input: &str, tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(relative) = lower[cursor..].find(&open) {
        let start = cursor + relative;
        let after = lower[start + open.len()..].chars().next();
        let is_boundary =
            after.is_none_or(|ch| ch.is_ascii_whitespace() || matches!(ch, '>' | '/'));
        if !is_boundary {
            out.push_str(&input[cursor..start + open.len()]);
            cursor = start + open.len();
            continue;
        }

        let Some(close_relative) = lower[start..].find(&close) else {
            // Không có thẻ đóng: bỏ phần còn lại để tránh lộ nội dung script/style.
            out.push_str(&input[cursor..start]);
            cursor = input.len();
            break;
        };
        let close_start = start + close_relative;
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
    ];

    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        let tail = &rest[position..];
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
    let without_scripts = remove_tag_blocks(html, "script");
    let without_styles = remove_tag_blocks(&without_scripts, "style");
    let text = replace_tags_with_space(&without_styles);
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
    fn truncates_on_utf8_boundary() {
        let text = "á".repeat(10);
        let (cut, truncated) = truncate_chars(&text, 5);
        assert_eq!(cut, "ááááá");
        assert!(truncated);

        let (full, truncated) = truncate_chars("ngắn", 10);
        assert_eq!(full, "ngắn");
        assert!(!truncated);
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
}
