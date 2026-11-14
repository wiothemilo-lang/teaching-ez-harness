use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tez_auth::connection;
use tez_core::agent::Agent;
use tez_core::context::{ContextBuilder, ContextInput};
use tez_core::subjects::{self, SubjectPack};
use tez_core::tools::balance_equation::BalanceEquation;
use tez_core::tools::calculator::Calculator;
use tez_core::tools::check_grammar::CheckGrammar;
use tez_core::tools::check_step::CheckStep;
use tez_core::tools::convert_units::ConvertUnits;
use tez_core::tools::molar_mass::MolarMass;
use tez_core::tools::physics_formula::PhysicsFormula;
use tez_core::tools::plot_function::PlotFunction;
use tez_core::tools::readability::Readability;
use tez_core::tools::ToolRegistry;
use tez_llm::{LlmProvider, Message, OpenAiCompatible};
use tez_store::{NewMessage, Store, UsageRecord};

#[derive(Debug, Deserialize)]
struct SaveConnectionRequest {
    provider: String,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
}

struct ResolvedConnection {
    provider_id: String,
    base_url: String,
    model: String,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct SubjectInfo {
    id: &'static str,
    name: &'static str,
    strategy: &'static str,
    tools: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ConnectionInfo {
    provider: String,
    model: String,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct AppStatus {
    connection: Option<ConnectionInfo>,
    subjects: Vec<SubjectInfo>,
    sessions: i64,
    data_dir: String,
    version: &'static str,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    subject: String,
    grade: String,
    message: String,
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionRequest {
    session_id: String,
}

#[derive(Debug, Serialize)]
struct StoredMessageInfo {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct SessionDetail {
    id: String,
    messages: Vec<StoredMessageInfo>,
}

#[derive(Debug, Serialize)]
struct ConnectionTest {
    ok: bool,
    message: String,
    latency_ms: u64,
}

#[derive(Debug, Serialize)]
struct ToolTrace {
    tool: String,
    args: String,
    output: String,
    ok: bool,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct ChatReply {
    session_id: String,
    text: String,
    tools: Vec<ToolTrace>,
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    id: String,
    subject: Option<String>,
    state: String,
    messages: i64,
    started_at: String,
    summary: Option<String>,
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "teaching-ez", "teaching-ez")
        .ok_or_else(|| anyhow::anyhow!("không xác định được thư mục dữ liệu"))?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn resolve_connection(dir: &Path) -> anyhow::Result<ResolvedConnection> {
    let saved = connection::load(dir)?.ok_or_else(|| {
        anyhow::anyhow!("Chưa kết nối provider. Mở Cài đặt để thêm API key hoặc chạy: tez connect --provider groq")
    })?;
    let api_key = connection::decrypt_key(dir, &saved)?;

    Ok(ResolvedConnection {
        provider_id: saved.provider,
        base_url: saved.base_url,
        model: saved.model,
        api_key,
    })
}

fn full_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Calculator);
    registry.register(CheckStep);
    registry.register(ConvertUnits);
    registry.register(Readability);
    registry.register(MolarMass);
    registry.register(BalanceEquation);
    registry.register(PlotFunction);
    registry.register(PhysicsFormula);
    registry.register(CheckGrammar);
    registry
}

fn registry_for(pack: &SubjectPack) -> ToolRegistry {
    full_registry().filtered(pack.tools)
}

fn subject_infos() -> Vec<SubjectInfo> {
    subjects::ALL
        .iter()
        .map(|pack| SubjectInfo {
            id: pack.id,
            name: pack.name,
            strategy: pack.strategy,
            tools: pack.tools.to_vec(),
        })
        .collect()
}

fn friendly(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[tauri::command]
fn app_status() -> Result<AppStatus, String> {
    let dir = data_dir().map_err(friendly)?;
    let connection = connection::load(&dir)
        .map_err(|error| error.to_string())?
        .map(|saved| ConnectionInfo {
            provider: saved.provider,
            model: saved.model,
            base_url: saved.base_url,
        });

    let sessions = Store::open(dir.join("tez.db"))
        .and_then(|store| store.session_count())
        .unwrap_or(0);

    Ok(AppStatus {
        connection,
        subjects: subject_infos(),
        sessions,
        data_dir: dir.display().to_string(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[tauri::command]
fn list_sessions(limit: Option<usize>) -> Result<Vec<SessionInfo>, String> {
    let dir = data_dir().map_err(friendly)?;
    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    let rows = store
        .list_sessions(limit.unwrap_or(20))
        .map_err(|error| error.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| SessionInfo {
            id: row.id,
            subject: row.subject,
            state: row.state,
            messages: row.messages,
            started_at: row.started_at,
            summary: row.summary,
        })
        .collect())
}

#[tauri::command]
fn load_session(request: SessionRequest) -> Result<SessionDetail, String> {
    let dir = data_dir().map_err(friendly)?;
    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    if !store
        .session_exists(&request.session_id)
        .map_err(|error| error.to_string())?
    {
        return Err(format!("session '{}' không tồn tại", request.session_id));
    }

    let messages = store
        .load_messages(&request.session_id, 200)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|message| StoredMessageInfo {
            role: message.role,
            content: message.content,
        })
        .collect();

    Ok(SessionDetail {
        id: request.session_id,
        messages,
    })
}

#[tauri::command]
fn save_connection(request: SaveConnectionRequest) -> Result<ConnectionInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    let provider = request.provider.trim().to_lowercase();
    let typed_key = request
        .api_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());

    let api_key = match typed_key {
        Some(key) => key,
        None => {
            let existing = connection::load(&dir).map_err(|error| error.to_string())?;
            match existing.filter(|saved| saved.provider == provider) {
                Some(saved) => {
                    connection::decrypt_key(&dir, &saved).map_err(|error| error.to_string())?
                }
                None if provider == "ollama" || provider == "custom" => String::new(),
                None => {
                    return Err("API key không được để trống với provider này.".into());
                }
            }
        }
    };

    let saved = connection::save(
        &dir,
        &provider,
        request.base_url.as_deref(),
        request.model.as_deref(),
        &api_key,
    )
    .map_err(|error| error.to_string())?;

    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    store
        .ensure_profile("default", "Học sinh")
        .map_err(|error| error.to_string())?;

    Ok(ConnectionInfo {
        provider: saved.provider,
        model: saved.model,
        base_url: saved.base_url,
    })
}

#[tauri::command]
async fn test_connection() -> Result<ConnectionTest, String> {
    test_connection_inner().await.map_err(friendly)
}

async fn test_connection_inner() -> anyhow::Result<ConnectionTest> {
    let dir = data_dir()?;
    let saved = resolve_connection(&dir)?;
    let provider = OpenAiCompatible::new(&saved.provider_id, &saved.base_url, &saved.api_key);
    let started = Instant::now();
    let message = tez_llm::ping(&provider, &saved.model).await?;
    let latency_ms = started.elapsed().as_millis() as u64;

    Ok(ConnectionTest {
        ok: true,
        message: message.trim().to_string(),
        latency_ms,
    })
}

#[tauri::command]
async fn chat(request: ChatRequest) -> Result<ChatReply, String> {
    chat_inner(request).await.map_err(friendly)
}

async fn chat_inner(request: ChatRequest) -> anyhow::Result<ChatReply> {
    let dir = data_dir()?;
    let pack = subjects::resolve(&request.subject)
        .ok_or_else(|| anyhow::anyhow!("môn không hỗ trợ: {}", request.subject))?;
    let connection = resolve_connection(&dir)?;

    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Học sinh")?;

    let (session_id, history) = match request.session_id.as_deref() {
        Some(id) => {
            if !store.session_exists(id)? {
                anyhow::bail!("session '{id}' không tồn tại");
            }
            store.resume_session(id)?;
            let history = store
                .load_messages(id, 20)?
                .into_iter()
                .filter_map(|message| match message.role.as_str() {
                    "user" => Some(Message::user(message.content)),
                    "assistant" => Some(Message::assistant(message.content)),
                    _ => None,
                })
                .collect();
            (id.to_string(), history)
        }
        None => (
            store.create_session("default", None, Some(pack.id))?,
            Vec::new(),
        ),
    };

    let hits = store.search_chunks(&request.message, Some(pack.id), 4)?;
    let excerpts: Vec<String> = hits
        .iter()
        .map(|hit| format!("[{}] {}", hit.title, hit.snippet))
        .collect();

    store.append_message(NewMessage {
        session_id: &session_id,
        role: "user",
        content: &request.message,
        model: None,
        tokens_in: 0,
        tokens_out: 0,
    })?;
    drop(store);

    let mut recent = history;
    recent.push(Message::user(request.message.clone()));

    let objectives: Vec<String> = Vec::new();
    let messages = ContextBuilder.build(ContextInput {
        subject: pack,
        grade: &request.grade,
        objectives: &objectives,
        excerpts: &excerpts,
        student_summary: None,
        state: "practice",
        recent: &recent,
    });

    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    ));
    let agent = Agent::new(provider, registry_for(pack));
    let started = Instant::now();
    let outcome = agent.run(&connection.model, messages).await?;
    let latency_ms = started.elapsed().as_millis() as u64;

    let store = Store::open(dir.join("tez.db"))?;
    store.append_message(NewMessage {
        session_id: &session_id,
        role: "assistant",
        content: &outcome.text,
        model: Some(&connection.model),
        tokens_in: outcome.usage.input_tokens,
        tokens_out: outcome.usage.output_tokens,
    })?;
    for step in &outcome.steps {
        store.record_tool_call(
            &session_id,
            &step.tool,
            &step.args,
            &step.output,
            step.ok,
            step.duration_ms,
        )?;
    }
    store.record_usage(UsageRecord {
        profile_id: "default",
        connection_id: None,
        provider_id: &connection.provider_id,
        model_id: &connection.model,
        task: "chat",
        tokens_in: outcome.usage.input_tokens,
        tokens_out: outcome.usage.output_tokens,
        latency_ms: Some(latency_ms),
        status: "ok",
    })?;
    store.resume_session(&session_id)?;
    drop(store);

    Ok(ChatReply {
        session_id,
        text: outcome.text,
        tools: outcome
            .steps
            .iter()
            .map(|step| ToolTrace {
                tool: step.tool.clone(),
                args: step.args.to_string(),
                output: step.output.clone(),
                ok: step.ok,
                duration_ms: step.duration_ms,
            })
            .collect(),
        input_tokens: outcome.usage.input_tokens,
        output_tokens: outcome.usage.output_tokens,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_status,
            chat,
            list_sessions,
            load_session,
            save_connection,
            test_connection
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
