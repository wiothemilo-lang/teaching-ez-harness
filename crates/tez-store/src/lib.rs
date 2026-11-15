use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use rusqlite::functions::FunctionFlags;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

/// SQLite local store. Một file DB cho mỗi máy.
pub struct Store {
    conn: Connection,
    path: PathBuf,
}

/// Dữ liệu để ghi một message vào session.
pub struct NewMessage<'a> {
    pub session_id: &'a str,
    pub role: &'a str,
    pub content: &'a str,
    pub model: Option<&'a str>,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// Một dòng usage để tính chi phí/token về sau.
pub struct UsageRecord<'a> {
    pub profile_id: &'a str,
    pub connection_id: Option<&'a str>,
    pub provider_id: &'a str,
    pub model_id: &'a str,
    pub task: &'a str,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub latency_ms: Option<u64>,
    pub status: &'a str,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub subject: Option<String>,
    pub mode: String,
    pub state: String,
    pub summary: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub messages: i64,
}

#[derive(Debug, Clone, Default)]
pub struct UsageSummary {
    pub requests: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

#[derive(Debug, Clone)]
pub struct DocumentRow {
    pub id: String,
    pub title: String,
    pub subject: Option<String>,
    pub source_path: Option<String>,
    pub bytes: i64,
    pub chunks: i64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub subject: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub role: String,
    pub content: String,
    /// Data URL ảnh đính kèm (tối đa do client giới hạn).
    pub images: Vec<String>,
}

const SCHEMA_VERSION: i64 = 5;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    age_mode TEXT NOT NULL DEFAULT 'teen',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS connections (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    label TEXT NOT NULL,
    auth_type TEXT NOT NULL,
    encrypted_payload TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS lessons (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    title TEXT NOT NULL,
    subject TEXT NOT NULL,
    grade_level TEXT,
    objectives TEXT NOT NULL DEFAULT '[]',
    strategy TEXT NOT NULL DEFAULT 'socratic',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    lesson_id TEXT,
    profile_id TEXT NOT NULL,
    subject TEXT,
    mode TEXT NOT NULL DEFAULT 'chat',
    state TEXT NOT NULL DEFAULT 'intake',
    summary TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at TEXT
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    images TEXT NOT NULL DEFAULT '[]',
    model TEXT,
    tokens_in INTEGER,
    tokens_out INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS context_snapshots (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    step_no INTEGER NOT NULL,
    sections_json TEXT NOT NULL,
    tokens_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    args_json TEXT NOT NULL,
    output TEXT NOT NULL,
    ok INTEGER NOT NULL,
    duration_ms INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS session_todos (
    session_id TEXT PRIMARY KEY,
    items_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS ai_usage (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    connection_id TEXT,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    task TEXT,
    tokens_in INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER,
    status TEXT NOT NULL DEFAULT 'ok',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL DEFAULT 'default',
    subject TEXT,
    title TEXT NOT NULL,
    source_path TEXT,
    mime TEXT,
    bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS document_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    chunk_no INTEGER NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS document_chunks_fts USING fts5(
    content,
    search_text,
    chunk_id UNINDEXED,
    document_id UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_ai_usage_created ON ai_usage(created_at);
CREATE INDEX IF NOT EXISTS idx_document_chunks_doc ON document_chunks(document_id, chunk_no);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
INSERT OR IGNORE INTO schema_migrations(version) VALUES (2);
INSERT OR IGNORE INTO schema_migrations(version) VALUES (3);
INSERT OR IGNORE INTO schema_migrations(version) VALUES (4);
INSERT OR IGNORE INTO schema_migrations(version) VALUES (5);
"#;

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        subject: row.get(1)?,
        mode: row.get(2)?,
        state: row.get(3)?,
        summary: row.get(4)?,
        started_at: row.get(5)?,
        ended_at: row.get(6)?,
        messages: row.get(7)?,
    })
}

fn create_private_file(path: &Path) -> Result<(), StoreError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    drop(file);
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        Ok(()) => Ok(()),
        // -wal/-shm là file tạm, có thể biến mất giữa lúc kiểm tra và chmod.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

fn ensure_wal_mode(conn: &Connection) -> Result<(), StoreError> {
    // Serialize trong process để nhiều thread mở DB lần đầu không tranh nhau
    // đổi journal_mode; process khác vẫn được xử lý bằng retry bên dưới.
    static JOURNAL_MODE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = JOURNAL_MODE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let current: String =
        retry_busy(|| Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?))?;
    if current.eq_ignore_ascii_case("wal") {
        return Ok(());
    }

    retry_busy(|| {
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        if mode.eq_ignore_ascii_case("wal") {
            Ok(())
        } else {
            Err(StoreError::InvalidInput(format!(
                "không bật được WAL (journal_mode={mode})"
            )))
        }
    })
}

fn is_busy_error(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _))
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// SQLite có thể trả SQLITE_BUSY ngay cả khi đã có busy_timeout (đặc biệt lúc
/// đổi journal_mode hoặc BEGIN IMMEDIATE); thử lại vài lần trước khi bỏ cuộc.
fn retry_busy<T>(mut operation: impl FnMut() -> Result<T, StoreError>) -> Result<T, StoreError> {
    const ATTEMPTS: usize = 50;
    for attempt in 0..ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt + 1 < ATTEMPTS && is_busy_error(&error) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("retry_busy luôn trả về trong vòng lặp")
}

const IMAGE_REF_PREFIX: &str = "tez-image:";

fn image_component(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    if out.is_empty() {
        out.push_str("image");
    }
    out
}

fn image_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "bin",
    }
}

fn image_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// Tách data URL base64 thành mime + bytes. Trả `None` nếu không phải base64.
fn decode_data_url(value: &str) -> Option<(String, Vec<u8>)> {
    let rest = value.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    let mime = meta.split(';').next().unwrap_or("").trim();
    let mime = if mime.contains('/') {
        mime
    } else {
        "application/octet-stream"
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    Some((mime.to_string(), bytes))
}

impl Store {
    fn image_root(&self) -> Option<PathBuf> {
        if self.path == Path::new(":memory:") {
            return None;
        }
        Some(
            self.path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("images"),
        )
    }

    /// Ghi ảnh ra file riêng và trả về tham chiếu `tez-image:<relative>`.
    /// Nếu không giải mã/ghi được thì giữ nguyên data URL để không mất ảnh.
    fn persist_images(&self, message_id: &str, session_id: &str, images: &[String]) -> Vec<String> {
        let Some(root) = self.image_root() else {
            return images.to_vec();
        };
        let dir = root.join(image_component(session_id));
        let mut stored = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let Some((mime, bytes)) = decode_data_url(image) else {
                stored.push(image.clone());
                continue;
            };
            let name = format!(
                "{}-{index}.{}",
                image_component(message_id),
                image_extension(&mime)
            );
            let path = dir.join(&name);
            if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&path, &bytes).is_err() {
                stored.push(image.clone());
                continue;
            }
            let _ = set_private_permissions(&path);
            let base = root.parent().unwrap_or_else(|| Path::new("."));
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            stored.push(format!("{IMAGE_REF_PREFIX}{relative}"));
        }
        stored
    }

    fn resolve_images(&self, images: &[String]) -> Vec<String> {
        let Some(root) = self.image_root() else {
            return images.to_vec();
        };
        let base = root.parent().unwrap_or_else(|| Path::new("."));
        images
            .iter()
            .map(|image| {
                let Some(relative) = image.strip_prefix(IMAGE_REF_PREFIX) else {
                    return image.clone();
                };
                let path = base.join(relative);
                let Ok(bytes) = std::fs::read(&path) else {
                    return image.clone();
                };
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                format!("data:{};base64,{encoded}", image_mime(&path))
            })
            .collect()
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let in_memory = path == Path::new(":memory:");
        if !in_memory {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Tạo file với mode 0600 trước khi SQLite mở: SQLite sao chép mode này
            // cho -wal/-shm, nên không còn file DB lộ quyền 0644.
            create_private_file(&path)?;
        }

        let conn = Connection::open(&path)?;
        // busy_timeout phải có hiệu lực trước khi đổi journal_mode.
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")?;
        if !in_memory {
            ensure_wal_mode(&conn)?;
        }

        let store = Self { conn, path };
        store.harden_permissions()?;
        store.migrate()?;
        // DDL có thể vừa tạo -wal/-shm; siết quyền lần cuối.
        store.harden_permissions()?;
        Ok(store)
    }

    /// Mở DB đã tồn tại ở chế độ chỉ đọc: không tạo file, không migrate.
    ///
    /// Dùng cho lệnh chẩn đoán như `tez doctor` để không làm thay đổi dữ liệu.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { conn, path })
    }

    fn migrate(&self) -> Result<(), StoreError> {
        // DB đã migrate thì chỉ đọc user_version, không lấy khóa ghi.
        if Self::user_version(&self.conn)? >= SCHEMA_VERSION {
            return Ok(());
        }

        retry_busy(|| {
            // BEGIN IMMEDIATE: hai process mở DB cùng lúc không thể cùng ALTER/CREATE.
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            match self.apply_migrations() {
                Ok(()) => match self.conn.execute_batch("COMMIT") {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        let _ = self.conn.execute_batch("ROLLBACK");
                        Err(error.into())
                    }
                },
                Err(error) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(error)
                }
            }
        })
    }

    fn apply_migrations(&self) -> Result<(), StoreError> {
        // Process khác có thể đã migrate xong trong lúc ta chờ khóa.
        if Self::user_version(&self.conn)? >= SCHEMA_VERSION {
            return Ok(());
        }

        self.conn.execute_batch(SCHEMA)?;
        Self::add_column_if_missing(&self.conn, "sessions", "subject", "TEXT")?;
        Self::add_column_if_missing(
            &self.conn,
            "sessions",
            "mode",
            "TEXT NOT NULL DEFAULT 'chat'",
        )?;
        Self::add_column_if_missing(&self.conn, "documents", "subject", "TEXT")?;
        Self::add_column_if_missing(
            &self.conn,
            "messages",
            "images",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        Self::ensure_fts_schema(&self.conn)?;
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        Ok(())
    }

    fn user_version(conn: &Connection) -> Result<i64, StoreError> {
        Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// FTS5 không hỗ trợ ALTER; nếu bảng cũ thiếu `search_text` thì tạo lại + reindex.
    fn ensure_fts_schema(conn: &Connection) -> Result<(), StoreError> {
        let mut statement = conn.prepare("PRAGMA table_info(document_chunks_fts)")?;
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(1))?
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);

        if columns.iter().any(|column| column == "search_text") {
            return Ok(());
        }

        conn.execute_batch(
            "DROP TABLE IF EXISTS document_chunks_fts;
             CREATE VIRTUAL TABLE document_chunks_fts USING fts5(
                content,
                search_text,
                chunk_id UNINDEXED,
                document_id UNINDEXED,
                tokenize = 'unicode61 remove_diacritics 2'
             );",
        )?;

        // Một INSERT...SELECT thay vì từng dòng; lỗi được trả về thay vì bỏ qua.
        conn.create_scalar_function(
            "tez_normalize_search",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                let content: String = context.get(0)?;
                Ok(normalize_for_search(&content))
            },
        )?;
        conn.execute(
            "INSERT INTO document_chunks_fts (content, search_text, chunk_id, document_id)
             SELECT content, tez_normalize_search(content), id, document_id
             FROM document_chunks",
            [],
        )?;
        Ok(())
    }

    fn harden_permissions(&self) -> Result<(), StoreError> {
        if self.path == Path::new(":memory:") {
            return Ok(());
        }
        set_private_permissions(&self.path)?;
        set_private_permissions(&self.wal_path())?;
        set_private_permissions(&self.shm_path())?;
        Ok(())
    }

    /// SQLite không có `ADD COLUMN IF NOT EXISTS`; kiểm tra pragma trước.
    fn add_column_if_missing(
        conn: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<(), StoreError> {
        let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == column);
        drop(statement);

        if !exists {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Kiểm tra toàn vẹn nhanh; trả về `"ok"` hoặc mô tả lỗi của SQLite.
    pub fn quick_check(&self) -> Result<String, StoreError> {
        let result: String = self
            .conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        Ok(result)
    }

    /// Sao lưu DB đang mở sang file khác bằng SQLite backup API.
    ///
    /// API này đọc nhất quán cả nội dung trong WAL, thay vì copy `tez.db` đơn
    /// thuần (có thể thiếu giao dịch mới nhất hoặc dính trạng thái dở dang).
    pub fn backup_to(&self, path: impl AsRef<Path>) -> Result<(), StoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(path);
        create_private_file(path)?;

        let result = (|| {
            let mut destination = Connection::open(path)?;
            let backup = rusqlite::backup::Backup::new(&self.conn, &mut destination)?;
            backup.run_to_completion(64, Duration::from_millis(10), None)?;
            Ok::<(), StoreError>(())
        })();

        if result.is_err() {
            let _ = std::fs::remove_file(path);
            return result;
        }
        set_private_permissions(path)?;
        Ok(())
    }

    /// Dung lượng file DB (gồm cả WAL và SHM nếu có).
    pub fn size_bytes(&self) -> Result<u64, StoreError> {
        if self.path == Path::new(":memory:") {
            return Ok(0);
        }
        let mut total = std::fs::metadata(&self.path)?.len();
        for path in [self.wal_path(), self.shm_path()] {
            match std::fs::metadata(&path) {
                Ok(metadata) => total += metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(total)
    }

    /// `tez.sqlite` -> `tez.sqlite-wal` (không thay phần mở rộng).
    fn wal_path(&self) -> PathBuf {
        self.sidecar_path("-wal")
    }

    /// `tez.sqlite` -> `tez.sqlite-shm`.
    fn shm_path(&self) -> PathBuf {
        self.sidecar_path("-shm")
    }

    fn sidecar_path(&self, suffix: &str) -> PathBuf {
        let mut name = self.path.as_os_str().to_os_string();
        name.push(suffix);
        PathBuf::from(name)
    }

    pub fn ensure_profile(&self, id: &str, name: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO profiles (id, name, age_mode) VALUES (?1, ?2, 'teen')",
            params![id, name],
        )?;
        Ok(())
    }

    pub fn create_session(
        &self,
        profile_id: &str,
        lesson_id: Option<&str>,
        subject: Option<&str>,
    ) -> Result<String, StoreError> {
        self.create_session_mode(profile_id, lesson_id, subject, "chat")
    }

    /// Tạo session kèm chế độ (`chat` hoặc `code`).
    pub fn create_session_mode(
        &self,
        profile_id: &str,
        lesson_id: Option<&str>,
        subject: Option<&str>,
        mode: &str,
    ) -> Result<String, StoreError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO sessions (id, lesson_id, profile_id, subject, mode)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, lesson_id, profile_id, subject, mode],
        )?;
        Ok(id)
    }

    pub fn append_message(&self, message: NewMessage<'_>) -> Result<String, StoreError> {
        self.append_message_with_images(message, &[])
    }

    /// Như `append_message`, kèm ảnh dạng data URL để resume không mất ảnh.
    pub fn append_message_with_images(
        &self,
        message: NewMessage<'_>,
        images: &[String],
    ) -> Result<String, StoreError> {
        let id = uuid::Uuid::new_v4().to_string();
        let stored_images = self.persist_images(&id, message.session_id, images);
        let images_json = serde_json::to_string(&stored_images)?;
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO messages (id, session_id, role, content, images, model, tokens_in, tokens_out)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                message.session_id,
                message.role,
                message.content,
                images_json,
                message.model,
                message.tokens_in as i64,
                message.tokens_out as i64,
            ],
        )?;
        // Session có message đầu tiên không còn là "intake" nữa.
        transaction.execute(
            "UPDATE sessions SET state = 'active' WHERE id = ?1 AND state = 'intake'",
            params![message.session_id],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn record_tool_call(
        &self,
        session_id: &str,
        tool_name: &str,
        args: &Value,
        output: &str,
        ok: bool,
        duration_ms: u64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO tool_calls (id, session_id, tool_name, args_json, output, ok, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uuid::Uuid::new_v4().to_string(),
                session_id,
                tool_name,
                args.to_string(),
                output,
                i64::from(ok),
                duration_ms as i64,
            ],
        )?;
        Ok(())
    }

    /// Lưu kế hoạch todo của phiên (JSON array).
    pub fn save_todos(&self, session_id: &str, items_json: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO session_todos (session_id, items_json, updated_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET
                items_json = excluded.items_json,
                updated_at = datetime('now')",
            params![session_id, items_json],
        )?;
        Ok(())
    }

    pub fn load_todos(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        let value = self
            .conn
            .query_row(
                "SELECT items_json FROM session_todos WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(value)
    }

    pub fn record_usage(&self, record: UsageRecord<'_>) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO ai_usage
                (id, profile_id, connection_id, provider_id, model_id, task, tokens_in, tokens_out, latency_ms, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                uuid::Uuid::new_v4().to_string(),
                record.profile_id,
                record.connection_id,
                record.provider_id,
                record.model_id,
                record.task,
                record.tokens_in as i64,
                record.tokens_out as i64,
                record.latency_ms.map(|value| value as i64),
                record.status,
            ],
        )?;
        Ok(())
    }

    pub fn end_session(&self, session_id: &str, summary: Option<&str>) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE sessions
             SET ended_at = datetime('now'), state = 'ended', summary = COALESCE(?2, summary)
             WHERE id = ?1",
            params![session_id, summary],
        )?;
        Ok(())
    }

    /// Đổi mode của session khi người dùng chuyển Plan ↔ Code mà vẫn giữ mạch.
    pub fn update_session_mode(&self, session_id: &str, mode: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE sessions SET mode = ?2 WHERE id = ?1",
            params![session_id, mode],
        )?;
        Ok(())
    }

    pub fn session_exists(&self, session_id: &str) -> Result<bool, StoreError> {
        let exists: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    /// Metadata của một session, dùng khi resume để khôi phục mode/môn học.
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT s.id, s.subject, s.mode, s.state, s.summary, s.started_at, s.ended_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id)
             FROM sessions s
             WHERE s.id = ?1",
        )?;
        let mut rows = statement.query_map(params![session_id], session_from_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Dọn session khi một lượt chat thất bại: xoá nếu chưa có message nào,
    /// ngược lại đánh dấu `failed` để không kẹt ở trạng thái `active`.
    pub fn abandon_session(&self, session_id: &str) -> Result<(), StoreError> {
        let transaction = self.conn.unchecked_transaction()?;
        let removed = transaction.execute(
            "DELETE FROM sessions
             WHERE id = ?1
               AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.session_id = ?1)",
            params![session_id],
        )?;
        if removed > 0 {
            // Không để lại dữ liệu mồ côi của session vừa xoá.
            transaction.execute(
                "DELETE FROM session_todos WHERE session_id = ?1",
                params![session_id],
            )?;
            transaction.execute(
                "DELETE FROM tool_calls WHERE session_id = ?1",
                params![session_id],
            )?;
            transaction.execute(
                "DELETE FROM context_snapshots WHERE session_id = ?1",
                params![session_id],
            )?;
        } else {
            transaction.execute(
                "UPDATE sessions
                 SET state = 'failed', ended_at = datetime('now')
                 WHERE id = ?1 AND state IN ('intake', 'active')",
                params![session_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Mở lại session để chat tiếp.
    pub fn resume_session(&self, session_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE sessions SET state = 'active', ended_at = NULL WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Lấy N message gần nhất theo thứ tự thời gian.
    pub fn load_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT role, content, images FROM (
                SELECT role, content, images, rowid AS rid
                FROM messages
                WHERE session_id = ?1
                ORDER BY rowid DESC
                LIMIT ?2
             ) ORDER BY rid ASC",
        )?;
        let rows = statement.query_map(params![session_id, limit as i64], |row| {
            let images_json: String = row.get(2)?;
            let images = match serde_json::from_str::<Vec<String>>(&images_json) {
                Ok(images) => self.resolve_images(&images),
                Err(error) => {
                    eprintln!(
                        "[tez-store] images JSON hỏng, giữ message với danh sách ảnh rỗng: {error}"
                    );
                    Vec::new()
                }
            };
            Ok(StoredMessage {
                role: row.get(0)?,
                content: row.get(1)?,
                images,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    /// Thêm tài liệu văn bản, chia chunk và index FTS5.
    /// Trả về `(document_id, số chunk)`.
    pub fn add_document(
        &self,
        profile_id: &str,
        subject: Option<&str>,
        title: &str,
        source_path: Option<&str>,
        mime: Option<&str>,
        content: &str,
    ) -> Result<(String, usize), StoreError> {
        let chunks = chunk_text(content);
        if chunks.is_empty() {
            return Err(StoreError::InvalidInput("tài liệu rỗng".to_string()));
        }

        let document_id = uuid::Uuid::new_v4().to_string();
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO documents (id, profile_id, subject, title, source_path, mime, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                document_id,
                profile_id,
                subject,
                title,
                source_path,
                mime,
                content.len() as i64
            ],
        )?;

        for (index, chunk) in chunks.iter().enumerate() {
            let chunk_id = uuid::Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO document_chunks (id, document_id, chunk_no, content)
                 VALUES (?1, ?2, ?3, ?4)",
                params![chunk_id, document_id, index as i64, chunk],
            )?;
            transaction.execute(
                "INSERT INTO document_chunks_fts (content, search_text, chunk_id, document_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![chunk, normalize_for_search(chunk), chunk_id, document_id],
            )?;
        }
        transaction.commit()?;

        Ok((document_id, chunks.len()))
    }

    pub fn list_documents(&self, limit: usize) -> Result<Vec<DocumentRow>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT d.id, d.title, d.subject, d.source_path, d.bytes, d.created_at,
                    (SELECT COUNT(*) FROM document_chunks c WHERE c.document_id = d.id)
             FROM documents d
             ORDER BY d.created_at DESC, d.rowid DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(DocumentRow {
                id: row.get(0)?,
                title: row.get(1)?,
                subject: row.get(2)?,
                source_path: row.get(3)?,
                bytes: row.get(4)?,
                created_at: row.get(5)?,
                chunks: row.get(6)?,
            })
        })?;

        let mut documents = Vec::new();
        for row in rows {
            documents.push(row?);
        }
        Ok(documents)
    }

    /// Tìm chunk liên quan bằng FTS5, lọc theo môn (kèm tài liệu chung).
    pub fn search_chunks(
        &self,
        query: &str,
        subject: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, StoreError> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }

        let mut statement = self.conn.prepare(
            "SELECT c.id, c.document_id, d.title, d.subject,
                    snippet(document_chunks_fts, 1, '[', ']', '…', 16)
             FROM document_chunks_fts
             JOIN document_chunks c ON c.id = document_chunks_fts.chunk_id
             JOIN documents d ON d.id = c.document_id
             WHERE document_chunks_fts MATCH ?1
               AND (?2 IS NULL OR d.subject = ?2 OR d.subject IS NULL)
             ORDER BY bm25(document_chunks_fts)
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![fts, subject, limit as i64], |row| {
            Ok(SearchHit {
                chunk_id: row.get(0)?,
                document_id: row.get(1)?,
                title: row.get(2)?,
                subject: row.get(3)?,
                snippet: row.get(4)?,
            })
        })?;

        let mut hits = Vec::new();
        for row in rows {
            hits.push(row?);
        }
        Ok(hits)
    }

    pub fn delete_document(&self, document_id: &str) -> Result<bool, StoreError> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM document_chunks_fts WHERE document_id = ?1",
            params![document_id],
        )?;
        transaction.execute(
            "DELETE FROM document_chunks WHERE document_id = ?1",
            params![document_id],
        )?;
        let removed =
            transaction.execute("DELETE FROM documents WHERE id = ?1", params![document_id])?;
        transaction.commit()?;
        Ok(removed > 0)
    }

    pub fn document_count(&self) -> Result<i64, StoreError> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn document_bytes(&self) -> Result<i64, StoreError> {
        let bytes =
            self.conn
                .query_row("SELECT COALESCE(SUM(bytes), 0) FROM documents", [], |row| {
                    row.get(0)
                })?;
        Ok(bytes)
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRow>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT s.id, s.subject, s.mode, s.state, s.summary, s.started_at, s.ended_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id)
             FROM sessions s
             ORDER BY s.started_at DESC, s.rowid DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], session_from_row)?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn session_count(&self) -> Result<i64, StoreError> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn usage_summary(&self) -> Result<UsageSummary, StoreError> {
        let summary = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0)
             FROM ai_usage",
            [],
            |row| {
                Ok(UsageSummary {
                    requests: row.get(0)?,
                    tokens_in: row.get(1)?,
                    tokens_out: row.get(2)?,
                })
            },
        )?;
        Ok(summary)
    }
}

fn flush_chunk(current: &mut String, chunks: &mut Vec<String>) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        chunks.push(trimmed.to_string());
    }
    current.clear();
}

/// Chia văn bản theo đoạn, mỗi chunk tối đa 1200 ký tự (không phải byte).
fn chunk_text(content: &str) -> Vec<String> {
    const TARGET_CHARS: usize = 1200;
    let normalized = content.replace("\r\n", "\n");
    let mut chunks = Vec::new();
    let mut current = String::new();

    for paragraph in normalized.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        let paragraph_chars = paragraph.chars().count();
        if !current.is_empty() && current.chars().count() + paragraph_chars + 2 > TARGET_CHARS {
            flush_chunk(&mut current, &mut chunks);
        }
        if paragraph_chars > TARGET_CHARS {
            flush_chunk(&mut current, &mut chunks);
            let chars: Vec<char> = paragraph.chars().collect();
            for slice in chars.chunks(TARGET_CHARS) {
                chunks.push(slice.iter().collect());
            }
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(paragraph);
        }
    }
    flush_chunk(&mut current, &mut chunks);
    chunks
}

/// Bỏ dấu tiếng Việt để tìm kiếm không cần gõ dấu.
fn normalize_for_search(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a' | 'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ'
            | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'a',
            'e' | 'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => {
                'e'
            }
            'i' | 'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
            'o' | 'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ'
            | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'o',
            'u' | 'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => {
                'u'
            }
            'y' | 'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
            'd' | 'đ' => 'd',
            _ => c,
        })
        .collect()
}

/// Tạo truy vấn FTS5 an toàn từ câu người dùng: mỗi từ thành một phrase, nối bằng OR.
fn fts_query(input: &str) -> String {
    let tokens: Vec<String> = input
        .split_whitespace()
        .map(normalize_for_search)
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\""))
        .collect();
    tokens.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema_and_accepts_profile() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();

        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 5);

        let version: i64 = store
            .connection()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        store.ensure_profile("p1", "Học sinh").unwrap();

        let name: String = store
            .connection()
            .query_row("SELECT name FROM profiles WHERE id = 'p1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "Học sinh");
    }

    #[test]
    fn read_only_open_does_not_create_or_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        assert!(Store::open_read_only(&path).is_err());
        assert!(!path.exists());

        let store = Store::open(&path).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        drop(store);

        let read_only = Store::open_read_only(&path).unwrap();
        assert_eq!(read_only.session_count().unwrap(), 0);
    }

    #[test]
    fn records_session_messages_tools_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();

        let session_id = store.create_session("default", None, Some("math")).unwrap();
        store
            .append_message(NewMessage {
                session_id: &session_id,
                role: "user",
                content: "2+2 bằng mấy?",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &session_id,
                role: "assistant",
                content: "Bằng 4.",
                model: Some("mock"),
                tokens_in: 12,
                tokens_out: 4,
            })
            .unwrap();
        store
            .record_tool_call(
                &session_id,
                "calculate",
                &serde_json::json!({"expression": "2+2"}),
                "4",
                true,
                3,
            )
            .unwrap();
        store
            .record_usage(UsageRecord {
                profile_id: "default",
                connection_id: None,
                provider_id: "mock",
                model_id: "mock",
                task: "chat",
                tokens_in: 12,
                tokens_out: 4,
                latency_ms: Some(30),
                status: "ok",
            })
            .unwrap();
        store
            .end_session(&session_id, Some("Kiểm tra phép cộng"))
            .unwrap();

        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].messages, 2);
        assert_eq!(sessions[0].state, "ended");
        assert_eq!(sessions[0].mode, "chat");

        let usage = store.usage_summary().unwrap();
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.tokens_in, 12);
        assert_eq!(usage.tokens_out, 4);
    }

    #[test]
    fn stores_session_mode() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();

        let chat = store
            .create_session("default", None, Some("general"))
            .unwrap();
        let code = store
            .create_session_mode("default", None, Some("general"), "code")
            .unwrap();

        let sessions = store.list_sessions(10).unwrap();
        let by_id = |id: &str| sessions.iter().find(|row| row.id == id).unwrap();
        assert_eq!(by_id(&chat).mode, "chat");
        assert_eq!(by_id(&code).mode, "code");
    }

    #[test]
    fn stores_session_todos() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        let id = store
            .create_session("default", None, Some("general"))
            .unwrap();

        assert_eq!(store.load_todos(&id).unwrap(), None);
        store
            .save_todos(&id, r#"[{"content":"đọc file","status":"pending"}]"#)
            .unwrap();
        let saved = store.load_todos(&id).unwrap().unwrap();
        assert!(saved.contains("đọc file"));

        store
            .save_todos(&id, r#"[{"content":"đọc file","status":"completed"}]"#)
            .unwrap();
        assert!(store
            .load_todos(&id)
            .unwrap()
            .unwrap()
            .contains("completed"));
    }

    #[test]
    fn gets_session_metadata_for_resume() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();

        let id = store
            .create_session_mode("default", None, Some("math"), "code")
            .unwrap();
        let row = store
            .get_session(&id)
            .unwrap()
            .expect("session phải tồn tại");
        assert_eq!(row.id, id);
        assert_eq!(row.mode, "code");
        assert_eq!(row.subject.as_deref(), Some("math"));
        assert_eq!(row.state, "intake");
        assert_eq!(row.messages, 0);

        assert!(store.get_session("không-tồn-tại").unwrap().is_none());
    }

    #[test]
    fn updates_session_mode_on_plan_to_code() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Người dùng").unwrap();
        let id = store
            .create_session_mode("default", None, None, "plan")
            .unwrap();

        store.update_session_mode(&id, "code").unwrap();
        let row = store.get_session(&id).unwrap().unwrap();
        assert_eq!(row.mode, "code");
    }

    #[test]
    fn abandon_session_removes_empty_and_marks_used() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();

        let empty = store
            .create_session_mode("default", None, Some("general"), "chat")
            .unwrap();
        store.abandon_session(&empty).unwrap();
        assert!(store.get_session(&empty).unwrap().is_none());

        let used = store
            .create_session_mode("default", None, Some("general"), "chat")
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &used,
                role: "user",
                content: "hello",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();
        // append_message đã chuyển session sang active; lượt chat lỗi đánh dấu failed.
        store.abandon_session(&used).unwrap();
        let row = store.get_session(&used).unwrap().unwrap();
        assert_eq!(row.state, "failed");
        assert_eq!(row.messages, 1);
        assert!(row.ended_at.is_some());
    }

    #[test]
    fn abandon_session_removes_related_rows_of_empty_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        let id = store
            .create_session_mode("default", None, Some("general"), "chat")
            .unwrap();

        store
            .save_todos(&id, r#"[{"content":"đọc file","status":"pending"}]"#)
            .unwrap();
        store
            .record_tool_call(&id, "read_file", &serde_json::json!({}), "ok", true, 1)
            .unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO context_snapshots (id, session_id, step_no, sections_json, tokens_json)
                 VALUES ('snap-1', ?1, 1, '[]', '{}')",
                params![id],
            )
            .unwrap();

        store.abandon_session(&id).unwrap();
        assert!(store.get_session(&id).unwrap().is_none());

        for table in ["session_todos", "tool_calls", "context_snapshots"] {
            let count: i64 = store
                .connection()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1"),
                    params![id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} còn dữ liệu mồ côi");
        }
    }

    #[test]
    fn append_message_activates_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        let id = store
            .create_session_mode("default", None, Some("general"), "chat")
            .unwrap();
        assert_eq!(store.get_session(&id).unwrap().unwrap().state, "intake");

        store
            .append_message(NewMessage {
                session_id: &id,
                role: "user",
                content: "hello",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();
        assert_eq!(store.get_session(&id).unwrap().unwrap().state, "active");
    }

    #[test]
    fn persists_images_with_messages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Người dùng").unwrap();
        let id = store
            .create_session_mode("default", None, None, "chat")
            .unwrap();
        let images = vec!["data:image/png;base64,AAAA".to_string()];

        store
            .append_message_with_images(
                NewMessage {
                    session_id: &id,
                    role: "user",
                    content: "xem ảnh này",
                    model: None,
                    tokens_in: 0,
                    tokens_out: 0,
                },
                &images,
            )
            .unwrap();

        let loaded = store.load_messages(&id, 10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "xem ảnh này");
        assert_eq!(loaded[0].images, images);
    }

    #[test]
    fn stores_images_as_files_and_keeps_refs_in_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Người dùng").unwrap();
        let session_id = store
            .create_session_mode("default", None, None, "chat")
            .unwrap();
        let images = vec!["data:image/png;base64,AAAA".to_string()];
        store
            .append_message_with_images(
                NewMessage {
                    session_id: &session_id,
                    role: "user",
                    content: "xem ảnh này",
                    model: None,
                    tokens_in: 0,
                    tokens_out: 0,
                },
                &images,
            )
            .unwrap();

        let stored: String = store
            .conn
            .query_row(
                "SELECT images FROM messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored.contains("tez-image:"), "{stored}");
        assert!(!stored.contains("data:image"), "{stored}");

        let loaded = store.load_messages(&session_id, 10).unwrap();
        assert_eq!(loaded[0].images, images);

        let image_dir = dir.path().join("images").join(&session_id);
        assert!(image_dir.is_dir());
        assert_eq!(std::fs::read_dir(&image_dir).unwrap().count(), 1);
    }

    #[test]
    fn malformed_images_json_keeps_message_with_empty_images() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        store.ensure_profile("default", "Người dùng").unwrap();
        let id = store
            .create_session_mode("default", None, None, "chat")
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &id,
                role: "user",
                content: "vẫn giữ message",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE messages SET images = 'không-phải-json' WHERE session_id = ?1",
                params![id],
            )
            .unwrap();

        let loaded = store.load_messages(&id, 10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "vẫn giữ message");
        assert!(loaded[0].images.is_empty());
    }

    #[test]
    fn wal_path_keeps_original_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.sqlite");
        let store = Store::open(&path).unwrap();
        assert_eq!(store.wal_path(), dir.path().join("tez.sqlite-wal"));
    }

    #[test]
    fn supports_in_memory_database() {
        let store = Store::open(":memory:").unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        let id = store
            .create_session_mode("default", None, Some("math"), "chat")
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &id,
                role: "user",
                content: "trong bộ nhớ",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();

        assert_eq!(store.load_messages(&id, 10).unwrap().len(), 1);
        assert_eq!(store.size_bytes().unwrap(), 0);
    }

    #[test]
    fn concurrent_open_migrates_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    // 8 thread x 25 lần mở = 200 lần, đủ để tái hiện race cũ.
                    for _ in 0..25 {
                        Store::open(&path).expect("mở DB đồng thời phải thành công");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let store = Store::open(&path).unwrap();
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'mode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn opening_migrated_db_does_not_take_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        let store = Store::open(&path).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        // Giữ khóa ghi trên connection hiện tại: open() của DB đã migrate
        // không được chờ khóa này.
        store.connection().execute_batch("BEGIN IMMEDIATE").unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();
        let open_path = path.clone();
        let handle = std::thread::spawn(move || {
            let _ = sender.send(Store::open(open_path).map(|_| ()));
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));

        store.connection().execute_batch("ROLLBACK").unwrap();
        handle.join().unwrap();

        result
            .expect("open() không được chờ khóa ghi của DB đã migrate")
            .expect("open() phải thành công");
    }

    #[test]
    fn migrates_documents_subject_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        {
            let store = Store::open(&path).unwrap();
            store.ensure_profile("default", "Học sinh").unwrap();
        }
        {
            // Mô phỏng DB cũ trước khi có cột documents.subject.
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "ALTER TABLE documents DROP COLUMN subject;
                 PRAGMA user_version = 0;",
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        store
            .add_document("default", Some("math"), "Ghi chú", None, None, "nội dung")
            .unwrap();
        let documents = store.list_documents(10).unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].subject.as_deref(), Some("math"));
        let version: i64 = store
            .connection()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn rebuilds_legacy_fts_with_search_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        let content = "Định lý Pythagore: trong tam giác vuông, a² + b² = c².";
        {
            let store = Store::open(&path).unwrap();
            store
                .add_document("default", Some("math"), "Định lý", None, None, content)
                .unwrap();
        }
        {
            // Mô phỏng FTS cũ chỉ có cột content.
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE document_chunks_fts;
                 CREATE VIRTUAL TABLE document_chunks_fts USING fts5(
                    content,
                    chunk_id UNINDEXED,
                    document_id UNINDEXED,
                    tokenize = 'unicode61 remove_diacritics 2'
                 );
                 PRAGMA user_version = 0;",
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let search_text_columns: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('document_chunks_fts')
                 WHERE name = 'search_text'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(search_text_columns, 1);

        let hits = store.search_chunks("định", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("[dinh]"),
            "snippet = {}",
            hits[0].snippet
        );
    }

    #[test]
    fn fts_rebuild_propagates_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        {
            let store = Store::open(&path).unwrap();
            // BLOB trong cột TEXT khiến normalize_search không đọc được thành String.
            store
                .connection()
                .execute(
                    "INSERT INTO document_chunks (id, document_id, chunk_no, content)
                     VALUES ('chunk-1', 'doc-1', 0, X'00')",
                    [],
                )
                .unwrap();
        }
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE document_chunks_fts;
                 CREATE VIRTUAL TABLE document_chunks_fts USING fts5(
                    content,
                    chunk_id UNINDEXED,
                    document_id UNINDEXED
                 );
                 PRAGMA user_version = 0;",
            )
            .unwrap();
        }

        match Store::open(&path) {
            Ok(_) => panic!("rebuild FTS phải trả lỗi thay vì bỏ qua dòng hỏng"),
            Err(error) => assert!(matches!(error, StoreError::Sqlite(_))),
        }
    }

    #[test]
    fn indexes_and_searches_documents() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();

        let content = "Định lý Pythagore: trong tam giác vuông, a² + b² = c².\n\n\
                       Hệ quả: cạnh huyền là cạnh lớn nhất.";
        let (document_id, chunks) = store
            .add_document(
                "default",
                Some("math"),
                "Định lý Pythagore",
                Some("/tmp/pythagore.md"),
                Some("text/markdown"),
                content,
            )
            .unwrap();
        assert_eq!(chunks, 1);
        assert_eq!(store.document_count().unwrap(), 1);
        assert_eq!(store.document_bytes().unwrap(), content.len() as i64);

        let hits = store.search_chunks("Pythagore", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id, document_id);
        assert!(
            hits[0].snippet.contains("[pythagore]"),
            "snippet = {}",
            hits[0].snippet
        );

        let hits = store.search_chunks("tam giac", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);

        let hits = store.search_chunks("dinh ly", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);

        // Query có dấu phải highlight trên cột search_text (đã bỏ dấu).
        let hits = store.search_chunks("định", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("[dinh]"),
            "snippet = {}",
            hits[0].snippet
        );

        let other = store
            .search_chunks("Pythagore", Some("chemistry"), 5)
            .unwrap();
        assert!(other.is_empty());

        let documents = store.list_documents(10).unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].chunks, 1);

        assert!(store.delete_document(&document_id).unwrap());
        assert_eq!(store.document_count().unwrap(), 0);
        let chunk_count: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
                params![document_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(chunk_count, 0);
        let fts_count: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM document_chunks_fts WHERE document_id = ?1",
                params![document_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0);
        assert!(store
            .search_chunks("Pythagore", None, 5)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn splits_long_documents_into_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("tez.db")).unwrap();
        let paragraph = "Một đoạn văn dài dùng để kiểm tra chia chunk. ".repeat(40);
        let content = format!("{paragraph}\n\n{paragraph}\n\n{paragraph}");

        let (_, chunks) = store
            .add_document("default", None, "Tài liệu dài", None, None, &content)
            .unwrap();
        assert!(chunks >= 3, "chunks = {chunks}");
    }

    #[test]
    fn chunks_vietnamese_text_by_characters_not_bytes() {
        // 1300 ký tự 'đ' = 2600 byte; nếu giới hạn là byte thì chỉ cần 1 chunk.
        let content = "đ".repeat(1300);
        let chunks = chunk_text(&content);
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.chars().count())
                .sum::<usize>(),
            1300
        );
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 1200));
    }

    #[test]
    fn combines_vietnamese_paragraphs_by_char_count() {
        // Mỗi đoạn 500 ký tự nhưng 1000 byte; 500 + 2 + 500 = 1002 ký tự
        // nên phải gộp thành một chunk (cách tính byte cũ sẽ tách đôi).
        let paragraph = "đ".repeat(500);
        let content = format!("{paragraph}\n\n{paragraph}");
        let chunks = chunk_text(&content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chars().count(), 1002);
    }

    #[test]
    fn chunks_vietnamese_paragraphs_within_char_limit() {
        let paragraph = "Định lý Pythagore: a² + b² = c². ".repeat(60);
        let content = format!("{paragraph}\n\n{paragraph}\n\n{paragraph}");
        let chunks = chunk_text(&content);
        assert!(chunks.len() >= 6, "chunks = {}", chunks.len());
        for chunk in &chunks {
            let chars = chunk.chars().count();
            assert!(chars <= 1200, "chunk có {chars} ký tự");
        }
    }

    #[cfg(unix)]
    #[test]
    fn creates_private_database_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        let store = Store::open(&path).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        let id = store
            .create_session_mode("default", None, None, "chat")
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &id,
                role: "user",
                content: "ghi để tạo WAL",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();

        for file in [&path, &store.wal_path(), &store.shm_path()] {
            let mode = std::fs::metadata(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()))
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{} có mode {mode:o}", file.display());
        }
    }

    #[test]
    fn size_bytes_includes_wal_and_shm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        let store = Store::open(&path).unwrap();
        store.ensure_profile("default", "Học sinh").unwrap();
        let id = store
            .create_session_mode("default", None, None, "chat")
            .unwrap();
        store
            .append_message(NewMessage {
                session_id: &id,
                role: "user",
                content: "dữ liệu để WAL có nội dung",
                model: None,
                tokens_in: 0,
                tokens_out: 0,
            })
            .unwrap();

        let expected: u64 = [&path, &store.wal_path(), &store.shm_path()]
            .into_iter()
            .map(|file| {
                std::fs::metadata(file)
                    .unwrap_or_else(|error| panic!("{}: {error}", file.display()))
                    .len()
            })
            .sum();
        assert_eq!(store.size_bytes().unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn size_bytes_reports_io_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tez.db");
        let store = Store::open(&path).unwrap();
        std::fs::rename(&path, dir.path().join("moved.db")).unwrap();

        assert!(matches!(store.size_bytes(), Err(StoreError::Io(_))));
    }
}
