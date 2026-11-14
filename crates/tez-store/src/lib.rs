use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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
}

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

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )?;

        let store = Self { conn, path };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(SCHEMA)?;
        self.add_column_if_missing("sessions", "subject", "TEXT")?;
        self.add_column_if_missing("sessions", "mode", "TEXT NOT NULL DEFAULT 'chat'")?;
        self.add_column_if_missing("documents", "subject", "TEXT")?;
        self.ensure_fts_schema()?;
        Ok(())
    }

    /// FTS5 không hỗ trợ ALTER; nếu bảng cũ thiếu `search_text` thì tạo lại + reindex.
    fn ensure_fts_schema(&self) -> Result<(), StoreError> {
        let mut statement = self
            .conn
            .prepare("PRAGMA table_info(document_chunks_fts)")?;
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(1))?
            .filter_map(Result::ok)
            .collect();
        drop(statement);

        if columns.iter().any(|column| column == "search_text") {
            return Ok(());
        }

        self.conn.execute_batch(
            "DROP TABLE IF EXISTS document_chunks_fts;
             CREATE VIRTUAL TABLE document_chunks_fts USING fts5(
                content,
                search_text,
                chunk_id UNINDEXED,
                document_id UNINDEXED,
                tokenize = 'unicode61 remove_diacritics 2'
             );",
        )?;

        let chunks: Vec<(String, String, String)> = {
            let mut statement = self
                .conn
                .prepare("SELECT id, document_id, content FROM document_chunks")?;
            let rows =
                statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.filter_map(Result::ok).collect()
        };

        for (chunk_id, document_id, content) in chunks {
            self.conn.execute(
                "INSERT INTO document_chunks_fts (content, search_text, chunk_id, document_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    content,
                    normalize_for_search(&content),
                    chunk_id,
                    document_id
                ],
            )?;
        }
        Ok(())
    }

    /// SQLite không có `ADD COLUMN IF NOT EXISTS`; kiểm tra pragma trước.
    fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<(), StoreError> {
        let mut statement = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == column);
        drop(statement);

        if !exists {
            self.conn.execute_batch(&format!(
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

    /// Dung lượng file DB (gồm cả WAL nếu có).
    pub fn size_bytes(&self) -> u64 {
        let main = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        let wal_path = self.path.with_extension("db-wal");
        let wal = std::fs::metadata(wal_path).map(|m| m.len()).unwrap_or(0);
        main + wal
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
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO messages (id, session_id, role, content, model, tokens_in, tokens_out)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                message.session_id,
                message.role,
                message.content,
                message.model,
                message.tokens_in as i64,
                message.tokens_out as i64,
            ],
        )?;
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
        self.conn.execute(
            "DELETE FROM sessions
             WHERE id = ?1
               AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.session_id = ?1)",
            params![session_id],
        )?;
        self.conn.execute(
            "UPDATE sessions
             SET state = 'failed', ended_at = datetime('now')
             WHERE id = ?1 AND state IN ('intake', 'active')",
            params![session_id],
        )?;
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
            "SELECT role, content FROM (
                SELECT role, content, rowid AS rid
                FROM messages
                WHERE session_id = ?1
                ORDER BY rowid DESC
                LIMIT ?2
             ) ORDER BY rid ASC",
        )?;
        let rows = statement.query_map(params![session_id, limit as i64], |row| {
            Ok(StoredMessage {
                role: row.get(0)?,
                content: row.get(1)?,
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
                    snippet(document_chunks_fts, 0, '[', ']', '…', 16)
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
            "DELETE FROM document_chunks_fts
             WHERE chunk_id IN (SELECT id FROM document_chunks WHERE document_id = ?1)",
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

/// Chia văn bản theo đoạn, mỗi chunk khoảng 1200 byte.
fn chunk_text(content: &str) -> Vec<String> {
    const TARGET_BYTES: usize = 1200;
    let normalized = content.replace("\r\n", "\n");
    let mut chunks = Vec::new();
    let mut current = String::new();

    for paragraph in normalized.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if !current.is_empty() && current.len() + paragraph.len() + 2 > TARGET_BYTES {
            flush_chunk(&mut current, &mut chunks);
        }
        if paragraph.len() > TARGET_BYTES {
            flush_chunk(&mut current, &mut chunks);
            let chars: Vec<char> = paragraph.chars().collect();
            for slice in chars.chunks(TARGET_BYTES) {
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
        // Session mới vẫn ở trạng thái intake khi lượt chat lỗi.
        store.abandon_session(&used).unwrap();
        let row = store.get_session(&used).unwrap().unwrap();
        assert_eq!(row.state, "failed");
        assert_eq!(row.messages, 1);
        assert!(row.ended_at.is_some());
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
            conn.execute_batch("ALTER TABLE documents DROP COLUMN subject")
                .unwrap();
        }

        let store = Store::open(&path).unwrap();
        store
            .add_document("default", Some("math"), "Ghi chú", None, None, "nội dung")
            .unwrap();
        let documents = store.list_documents(10).unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].subject.as_deref(), Some("math"));
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
        assert!(hits[0].snippet.contains("Pythagore"));

        let hits = store.search_chunks("tam giac", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);

        let hits = store.search_chunks("dinh ly", Some("math"), 5).unwrap();
        assert_eq!(hits.len(), 1);

        let other = store
            .search_chunks("Pythagore", Some("chemistry"), 5)
            .unwrap();
        assert!(other.is_empty());

        let documents = store.list_documents(10).unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].chunks, 1);

        assert!(store.delete_document(&document_id).unwrap());
        assert_eq!(store.document_count().unwrap(), 0);
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
}
