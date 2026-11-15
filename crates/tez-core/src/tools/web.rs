//! Tool `web_fetch`: tải và đọc nội dung HTTP(S) trong giới hạn an toàn.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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
    let (mut url, mut pin) = validate_url(raw_url, allow_private).await?;
    let mut response = None;

    for _ in 0..=MAX_REDIRECTS {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT);
        if let Some((host, address)) = &pin {
            builder = builder.resolve(host, *address);
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
    let is_html = content_type
        .as_deref()
        .map(|value| value.to_ascii_lowercase().contains("html"))
        .unwrap_or(false);

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
async fn validate_url(
    raw: &str,
    allow_private: bool,
) -> Result<(reqwest::Url, Option<(String, SocketAddr)>), ToolError> {
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
    let mut pin = None;
    for address in addresses {
        if is_forbidden_ip(address.ip()) {
            return Err(ToolError::InvalidArgs(format!(
                "host '{host}' trỏ tới địa chỉ nội bộ ({})",
                address.ip()
            )));
        }
        if pin.is_none() {
            pin = Some((host.clone(), address));
        }
    }
    match pin {
        Some(pin) => Ok((url, Some(pin))),
        None => Err(ToolError::Execution(format!(
            "không phân giải được host '{host}'"
        ))),
    }
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
}
