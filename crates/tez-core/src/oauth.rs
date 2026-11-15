//! OAuth 2.1 + PKCE cho MCP HTTP.
//!
//! Token được lưu trong `oauth.json` (0600) trong data dir. Module này không mở
//! browser; desktop/CLI gọi [`authorize_url`] rồi [`wait_for_code`] và
//! [`exchange_code`]. Token hết hạn được refresh tự động khi kết nối.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:33418/callback";
const TOKEN_FILE: &str = "oauth.json";
const TOKEN_REFRESH_SKEW_SECS: u64 = 30;

#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cấu hình OAuth: {0}")]
    Config(String),
    #[error("OAuth: {0}")]
    Auth(String),
}

/// Cấu hình OAuth cho một MCP server HTTP.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    #[serde(default, alias = "authorizationUrl")]
    pub authorization_url: String,
    #[serde(default, alias = "tokenUrl")]
    pub token_url: String,
    #[serde(default, alias = "clientId")]
    pub client_id: String,
    /// Tên biến môi trường chứa client secret (khuyến nghị).
    #[serde(default, alias = "clientSecretEnv")]
    pub client_secret_env: Option<String>,
    /// Client secret trực tiếp. Chỉ dùng khi không thể dùng biến môi trường.
    #[serde(default, alias = "clientSecret")]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, alias = "redirectUri")]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: u64,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthTokenStatus {
    pub authorized: bool,
    pub expires_at: u64,
    pub has_refresh: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TokenStore {
    #[serde(default)]
    servers: HashMap<String, OAuthToken>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn token_key(server: &str, config: &OAuthConfig) -> String {
    format!("{}|{}", server.trim(), config.token_url.trim())
}

fn token_path(dir: &Path) -> PathBuf {
    dir.join(TOKEN_FILE)
}

fn load_store(dir: &Path) -> Result<TokenStore, OAuthError> {
    let path = token_path(dir);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(TokenStore::default()),
        Err(error) => Err(error.into()),
    }
}

fn save_store(dir: &Path, store: &TokenStore) -> Result<(), OAuthError> {
    std::fs::create_dir_all(dir)?;
    let path = token_path(dir);
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(store)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(&temp, &path)?;
    Ok(())
}

pub fn store_token(
    dir: &Path,
    server: &str,
    config: &OAuthConfig,
    token: OAuthToken,
) -> Result<(), OAuthError> {
    let mut store = load_store(dir)?;
    store.servers.insert(token_key(server, config), token);
    save_store(dir, &store)
}

pub fn remove_token(dir: &Path, server: &str, config: &OAuthConfig) -> Result<bool, OAuthError> {
    let mut store = load_store(dir)?;
    let removed = store.servers.remove(&token_key(server, config)).is_some();
    if removed {
        save_store(dir, &store)?;
    }
    Ok(removed)
}

pub fn load_token(dir: &Path, server: &str, config: &OAuthConfig) -> Option<OAuthToken> {
    load_store(dir)
        .ok()
        .and_then(|store| store.servers.get(&token_key(server, config)).cloned())
}

pub fn status(dir: &Path, server: &str, config: &OAuthConfig) -> OAuthTokenStatus {
    match load_store(dir) {
        Ok(store) => match store.servers.get(&token_key(server, config)) {
            Some(token) => OAuthTokenStatus {
                authorized: !token.access_token.trim().is_empty()
                    || token
                        .refresh_token
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
                expires_at: token.expires_at,
                has_refresh: token
                    .refresh_token
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
            },
            None => OAuthTokenStatus {
                authorized: false,
                expires_at: 0,
                has_refresh: false,
            },
        },
        Err(_) => OAuthTokenStatus {
            authorized: false,
            expires_at: 0,
            has_refresh: false,
        },
    }
}

pub fn redirect_uri(config: &OAuthConfig) -> String {
    config
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_REDIRECT_URI)
        .to_string()
}

fn client_secret(config: &OAuthConfig) -> Result<Option<String>, OAuthError> {
    if let Some(variable) = config
        .client_secret_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let value = std::env::var(variable).map_err(|_| {
            OAuthError::Config(format!(
                "thiếu biến môi trường {variable} cho OAuth client secret"
            ))
        })?;
        let value = value.trim().to_string();
        if value.is_empty() {
            return Err(OAuthError::Config(format!(
                "biến môi trường {variable} cho OAuth client secret đang rỗng"
            )));
        }
        return Ok(Some(value));
    }
    Ok(config
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

/// Sinh state + PKCE verifier cho một lượt authorize.
pub fn new_pkce() -> (String, String) {
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let state = Uuid::new_v4().simple().to_string();
    (state, verifier)
}

pub fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub fn authorize_url(
    config: &OAuthConfig,
    state: &str,
    verifier: &str,
) -> Result<String, OAuthError> {
    if config.authorization_url.trim().is_empty() {
        return Err(OAuthError::Config("thiếu authorization_url".into()));
    }
    if config.client_id.trim().is_empty() {
        return Err(OAuthError::Config("thiếu client_id".into()));
    }
    let mut url = reqwest::Url::parse(config.authorization_url.trim())
        .map_err(|error| OAuthError::Config(format!("authorization_url không hợp lệ: {error}")))?;
    let redirect = redirect_uri(config);
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", config.client_id.trim());
        query.append_pair("redirect_uri", &redirect);
        if !config.scopes.is_empty() {
            query.append_pair("scope", &config.scopes.join(" "));
        }
        query.append_pair("state", state);
        query.append_pair("code_challenge", &pkce_challenge(verifier));
        query.append_pair("code_challenge_method", "S256");
        if let Some(resource) = config
            .resource
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            query.append_pair("resource", resource);
        }
    }
    Ok(url.to_string())
}

async fn token_request(
    config: &OAuthConfig,
    form: Vec<(String, String)>,
) -> Result<OAuthToken, OAuthError> {
    if config.token_url.trim().is_empty() {
        return Err(OAuthError::Config("thiếu token_url".into()));
    }
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent("tez/0.1 (+local)")
        .build()?
        .post(config.token_url.trim())
        .form(&form)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        let preview: String = body.chars().take(500).collect();
        return Err(OAuthError::Auth(format!(
            "token endpoint HTTP {status}: {preview}"
        )));
    }
    let parsed: TokenResponse = serde_json::from_str(&body)?;
    let expires_at = now_secs() + parsed.expires_in.unwrap_or(3_600).max(1);
    Ok(OAuthToken {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at,
        token_type: parsed.token_type.unwrap_or_else(|| "Bearer".to_string()),
        scope: parsed.scope,
    })
}

pub async fn exchange_code(
    dir: &Path,
    server: &str,
    config: &OAuthConfig,
    code: &str,
    verifier: &str,
) -> Result<OAuthToken, OAuthError> {
    let mut form = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("code".to_string(), code.to_string()),
        ("redirect_uri".to_string(), redirect_uri(config)),
        ("client_id".to_string(), config.client_id.trim().to_string()),
        ("code_verifier".to_string(), verifier.to_string()),
    ];
    if let Some(secret) = client_secret(config)? {
        form.push(("client_secret".to_string(), secret));
    }
    if let Some(resource) = config
        .resource
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        form.push(("resource".to_string(), resource.to_string()));
    }
    let token = token_request(config, form).await?;
    store_token(dir, server, config, token.clone())?;
    Ok(token)
}

pub async fn refresh_token(
    dir: &Path,
    server: &str,
    config: &OAuthConfig,
    previous: &OAuthToken,
) -> Result<OAuthToken, OAuthError> {
    let refresh = previous
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthError::Auth("token hết hạn và không có refresh_token".into()))?;
    let mut form = vec![
        ("grant_type".to_string(), "refresh_token".to_string()),
        ("refresh_token".to_string(), refresh.to_string()),
        ("client_id".to_string(), config.client_id.trim().to_string()),
    ];
    if let Some(secret) = client_secret(config)? {
        form.push(("client_secret".to_string(), secret));
    }
    if let Some(resource) = config
        .resource
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        form.push(("resource".to_string(), resource.to_string()));
    }
    let mut token = token_request(config, form).await?;
    if token.refresh_token.is_none() {
        token.refresh_token = Some(refresh.to_string());
    }
    store_token(dir, server, config, token.clone())?;
    Ok(token)
}

pub async fn access_token(
    dir: &Path,
    server: &str,
    config: &OAuthConfig,
) -> Result<String, OAuthError> {
    let store = load_store(dir)?;
    let Some(token) = store.servers.get(&token_key(server, config)).cloned() else {
        return Err(OAuthError::Auth(format!(
            "server '{server}' chưa được cấp quyền OAuth"
        )));
    };
    if !token.access_token.trim().is_empty()
        && (token.expires_at == 0 || token.expires_at > now_secs() + TOKEN_REFRESH_SKEW_SECS)
    {
        return Ok(token.access_token);
    }
    let refreshed = refresh_token(dir, server, config, &token).await?;
    Ok(refreshed.access_token)
}

/// Chờ callback OAuth trên loopback. Chỉ nhận một request rồi đóng listener.
pub fn wait_for_code(
    redirect: &str,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, OAuthError> {
    let url = reqwest::Url::parse(redirect)
        .map_err(|error| OAuthError::Config(format!("redirect_uri không hợp lệ: {error}")))?;
    let host = url
        .host_str()
        .map(|value| value.trim_matches(['[', ']']).to_string())
        .ok_or_else(|| OAuthError::Config("redirect_uri thiếu host".into()))?;
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return Err(OAuthError::Config(
            "redirect_uri phải dùng loopback (127.0.0.1/localhost/::1)".into(),
        ));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| OAuthError::Config("redirect_uri cần port cụ thể".into()))?;
    let listener = TcpListener::bind((host.as_str(), port))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(OAuthError::Auth(
                "hết thời gian chờ callback OAuth (5 phút)".into(),
            ));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let code = handle_callback(&mut stream, expected_state)?;
                return Ok(code);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn handle_callback(stream: &mut TcpStream, expected_state: &str) -> Result<String, OAuthError> {
    let mut buffer = [0u8; 8_192];
    let read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let result = parse_callback_target(target, expected_state);
    let (status, body) = match &result {
        Ok(_) => (
            "200 OK",
            "<html><body><h3>tez: OAuth thành công</h3><p>Bạn có thể đóng tab này.</p></body></html>",
        ),
        Err(_) => (
            "400 Bad Request",
            "<html><body><h3>tez: OAuth thất bại</h3><p>Kiểm tra lại cấu hình hoặc thử lại.</p></body></html>",
        ),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    result
}

fn parse_callback_target(target: &str, expected_state: &str) -> Result<String, OAuthError> {
    let url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .map_err(|error| OAuthError::Auth(format!("callback không hợp lệ: {error}")))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            "error" => error = Some(value.to_string()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Err(OAuthError::Auth(format!("OAuth trả lỗi: {error}")));
    }
    if state.as_deref() != Some(expected_state) {
        return Err(OAuthError::Auth("state OAuth không khớp".into()));
    }
    code.filter(|value| !value.trim().is_empty())
        .ok_or_else(|| OAuthError::Auth("callback thiếu code".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> OAuthConfig {
        OAuthConfig {
            authorization_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            client_id: "client-1".into(),
            scopes: vec!["read".into(), "write".into()],
            ..OAuthConfig::default()
        }
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_contains_pkce_state_and_scope() {
        let url = authorize_url(&config(), "state-1", "verifier-1").unwrap();
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-1"));
        assert!(url.contains("state=state-1"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=read+write"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A33418%2Fcallback"));
    }

    #[test]
    fn token_store_roundtrip_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let config = config();
        let token = OAuthToken {
            access_token: "abc".into(),
            refresh_token: Some("r".into()),
            expires_at: now_secs() + 3_600,
            token_type: "Bearer".into(),
            scope: None,
        };
        store_token(dir.path(), "server", &config, token).unwrap();
        let token_status = status(dir.path(), "server", &config);
        assert!(token_status.authorized);
        assert!(token_status.has_refresh);
        assert!(remove_token(dir.path(), "server", &config).unwrap());
        assert!(!status(dir.path(), "server", &config).authorized);
    }

    #[tokio::test]
    async fn expired_token_without_refresh_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let config = config();
        store_token(
            dir.path(),
            "server",
            &config,
            OAuthToken {
                access_token: "old".into(),
                refresh_token: None,
                expires_at: 1,
                token_type: "Bearer".into(),
                scope: None,
            },
        )
        .unwrap();
        let error = access_token(dir.path(), "server", &config)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("refresh_token"));
    }

    #[test]
    fn parses_callback_code_and_validates_state() {
        assert_eq!(
            parse_callback_target("/callback?code=abc&state=s1", "s1").unwrap(),
            "abc"
        );
        assert!(parse_callback_target("/callback?code=abc&state=other", "s1").is_err());
        assert!(parse_callback_target("/callback?error=denied&state=s1", "s1").is_err());
    }
}
