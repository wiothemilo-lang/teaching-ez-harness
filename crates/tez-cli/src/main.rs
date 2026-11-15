use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use dialoguer::{Confirm, Password};
use directories::ProjectDirs;
use serde_json::Value;
use tez_auth::connection;
use tez_auth::providers::{self, ProviderBook, ProviderInput};
use tez_auth::Vault;
use tez_core::agent::{Agent, AgentConfig, RunContext};
use tez_core::compaction::{session_artifact_dir, CompactSettings, Compactor};
use tez_core::context::{ContextBuilder, ContextInput};
use tez_core::harness;
use tez_core::hooks::{self, HookRegistry};
use tez_core::mcp;
use tez_core::memory::{self, MemoryForget, MemorySave, MemorySearch, MemoryStore};
use tez_core::modes::{self, Mode, PLAN_TOOLS};
use tez_core::permission::{
    ApprovalRequest, Approver, AutoApprover, PermissionMode, PermissionPolicy,
};
use tez_core::plugins;
use tez_core::project;
use tez_core::skills::SkillRegistry;
use tez_core::subagent::{SubagentRuntime, TaskTool};
use tez_core::subjects::{self, SubjectPack};
use tez_core::todo::{TodoState, TodoWrite};
use tez_core::tools::balance_equation::BalanceEquation;
use tez_core::tools::calculator::Calculator;
use tez_core::tools::check_grammar::CheckGrammar;
use tez_core::tools::check_step::CheckStep;
use tez_core::tools::convert_units::ConvertUnits;
use tez_core::tools::molar_mass::MolarMass;
use tez_core::tools::physics_formula::PhysicsFormula;
use tez_core::tools::plot_function::PlotFunction;
use tez_core::tools::readability::Readability;
use tez_core::tools::skill::SkillTool;
use tez_core::tools::web::WebFetchTool;
use tez_core::tools::{code_registry, ToolError, ToolRegistry};
use tez_llm::{LlmError, LlmProvider, Message, OpenAiCompatible};
use tez_store::{NewMessage, Store, UsageRecord};

#[derive(Parser)]
#[command(
    name = "tez",
    version,
    about = "Teaching EZ harness — local-first AI tutor"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Kiểm tra môi trường, DB, vault, provider.
    Doctor,
    /// Kết nối provider (API key hoặc Ollama local).
    Connect(ConnectArgs),
    /// Kiểm tra provider/model/key đã lưu bằng một prompt ngắn.
    Test,
    /// Chat hoặc làm việc với code agent.
    Chat(ChatArgs),
    /// Kiểm tra bản cập nhật mới trên GitHub Releases.
    Update(UpdateArgs),
    /// Xem các môn học và tool được phép.
    Subject {
        #[command(subcommand)]
        command: SubjectCommand,
    },
    /// Liệt kê session đã lưu.
    Sessions(SessionsArgs),
    /// Thống kê token đã dùng.
    Usage,
    /// Quản lý tài liệu RAG (txt/md).
    Doc {
        #[command(subcommand)]
        command: DocCommand,
    },
    /// Liệt kê và test tool.
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Quản lý nhiều provider đã lưu.
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    /// Liệt kê skill Markdown.
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    /// Liệt kê/test MCP server.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Liệt kê plugin local.
    Plugins,
    /// Xem bộ nhớ dài hạn đã lưu.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Xem cấu hình harness.
    Harness,
}

#[derive(Subcommand)]
enum MemoryCommand {
    List,
    Search { query: String },
    Forget { name: String },
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// Xem các provider đã lưu và provider đang dùng.
    List,
    /// Chuyển provider đang dùng.
    Use { id: String },
    /// Xóa một provider đã lưu.
    Remove { id: String },
}

#[derive(Subcommand)]
enum SkillsCommand {
    List,
    Show { id: String },
}

#[derive(Subcommand)]
enum McpCommand {
    List,
    Test { name: Option<String> },
}

#[derive(Args)]
struct ConnectArgs {
    /// groq | openrouter | openai | gemini | ollama | custom
    #[arg(long, default_value = "groq")]
    provider: String,
    /// Base URL OpenAI-compatible (bắt buộc với custom).
    #[arg(long)]
    base_url: Option<String>,
    /// Model mặc định.
    #[arg(long)]
    model: Option<String>,
    /// API key. Không khuyến khích vì lộ trong shell history; bỏ trống để nhập ẩn.
    #[arg(long)]
    api_key: Option<String>,
}

#[derive(Args)]
struct ChatArgs {
    /// Môn học tùy chọn: math, physics, chemistry, english. Bỏ trống = tổng quát.
    #[arg(long, default_value = "general")]
    subject: String,
    /// Chế độ: chat (mặc định), plan hoặc code.
    #[arg(long, default_value = "chat")]
    mode: String,
    /// Thư mục làm việc cho chế độ plan/code (mặc định: thư mục hiện tại).
    #[arg(long)]
    workspace: Option<PathBuf>,
    #[arg(long, default_value = "10")]
    grade: String,
    /// Ghi đè provider đã lưu.
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    /// Chat tiếp trong session đã có.
    #[arg(long)]
    session: Option<String>,
    /// Số message cũ nạp lại khi resume.
    #[arg(long, default_value_t = 12)]
    history: usize,
    /// Tự động duyệt mọi thao tác cần permission (dùng cho CI/script).
    #[arg(long, default_value_t = false)]
    yes: bool,
    /// Ghi đè permission mode: read_only | strict | standard | trust.
    #[arg(long)]
    permission: Option<String>,
    /// Ghi đè số bước tối đa của agent.
    #[arg(long)]
    max_steps: Option<usize>,
    /// Tắt tự động nạp memory vào context.
    #[arg(long, default_value_t = false)]
    no_memory: bool,
    /// Câu hỏi; bỏ trống để vào chế độ chat tương tác (hoặc đọc stdin nếu có pipe).
    prompt: Option<String>,
}

#[derive(Args)]
struct SessionsArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args)]
struct UpdateArgs {
    /// Chỉ kiểm tra, không tải.
    #[arg(long, default_value_t = false)]
    check_only: bool,
    /// Tải asset phù hợp vào thư mục dữ liệu.
    #[arg(long, default_value_t = false)]
    download: bool,
}

#[derive(Subcommand)]
enum SubjectCommand {
    List,
    Show { id: String },
}

#[derive(Subcommand)]
enum DocCommand {
    /// Thêm file .txt/.md vào kho tài liệu.
    Add(DocAddArgs),
    /// Liệt kê tài liệu đã ingest.
    List(DocListArgs),
    /// Tìm trong tài liệu bằng FTS5.
    Search(DocSearchArgs),
    /// Xóa một tài liệu.
    Remove { id: String },
}

#[derive(Args)]
struct DocAddArgs {
    path: PathBuf,
    /// Gắn tài liệu cho một môn (math, physics, chemistry, english).
    #[arg(long)]
    subject: Option<String>,
    /// Tiêu đề hiển thị; mặc định lấy tên file.
    #[arg(long)]
    title: Option<String>,
}

#[derive(Args)]
struct DocListArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args)]
struct DocSearchArgs {
    query: String,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long, default_value_t = 8)]
    limit: usize,
}

#[derive(Subcommand)]
enum ToolsCommand {
    List {
        /// Chỉ hiện tool của một môn.
        #[arg(long)]
        subject: Option<String>,
    },
    Test {
        name: String,
        #[arg(long)]
        args: String,
        /// Kiểm tra theo allowlist của môn.
        #[arg(long)]
        subject: Option<String>,
    },
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "teaching-ez", "teaching-ez")
        .ok_or_else(|| anyhow::anyhow!("không xác định được thư mục dữ liệu"))?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < MB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{:.1} MB", value / MB)
    }
}

fn env_api_key(provider: &str) -> Option<String> {
    let name = match provider {
        "groq" => "GROQ_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        "cloudflare" => "CLOUDFLARE_API_TOKEN",
        _ => "TEZ_API_KEY",
    };
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Gợi ý hành động cho các lỗi LLM hay gặp, thay vì để lộ body 401/403 thô.
fn llm_error_hint(error: &LlmError) -> Option<String> {
    match error {
        LlmError::Api { status, .. } if *status == 401 || *status == 403 => Some(format!(
            "API key bị provider từ chối (HTTP {status}). Kiểm tra lại key bằng \
             `tez connect --provider <loại>` hoặc biến môi trường tương ứng."
        )),
        LlmError::Api { status, .. } if *status == 429 => Some(
            "Provider đang giới hạn tốc độ (HTTP 429). Chờ vài giây rồi thử lại, \
             hoặc đổi model/provider."
                .to_string(),
        ),
        LlmError::MissingKey => Some(
            "Chưa có API key cho provider này. Chạy `tez connect --provider <loại>` \
             hoặc dùng --provider ollama/custom nếu endpoint không cần key."
                .to_string(),
        ),
        LlmError::Http(err) if err.is_timeout() => Some(
            "Provider không phản hồi kịp (timeout). Kiểm tra mạng/base_url rồi thử lại."
                .to_string(),
        ),
        _ => None,
    }
}

fn with_llm_hint(error: anyhow::Error) -> anyhow::Error {
    match error.downcast_ref::<LlmError>().and_then(llm_error_hint) {
        Some(hint) => error.context(hint),
        None => error,
    }
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

fn registry_for_mode(
    pack: &SubjectPack,
    mode: Mode,
    workspace: &Path,
) -> anyhow::Result<ToolRegistry> {
    match mode {
        Mode::Chat => Ok(registry_for(pack)),
        Mode::Plan => Ok(code_registry(workspace)?.filtered(PLAN_TOOLS)),
        Mode::Code => Ok(code_registry(workspace)?),
    }
}

fn resolve_subject(input: &str) -> anyhow::Result<&'static SubjectPack> {
    subjects::resolve(input).ok_or_else(|| {
        let names: Vec<&str> = subjects::ALL.iter().map(|pack| pack.name).collect();
        anyhow::anyhow!(
            "môn '{}' chưa hỗ trợ. Các môn hiện có: {}",
            input,
            names.join(", ")
        )
    })
}

async fn connect(args: ConnectArgs) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let provider = args.provider.trim().to_lowercase();
    connection::validate_provider(&provider)?;
    let api_key = match args.api_key {
        Some(key) => key,
        None if !connection::requires_api_key(&provider) => String::new(),
        None => {
            if let Some(key) = env_api_key(&provider) {
                key
            } else if std::io::stdin().is_terminal() {
                Password::new()
                    .with_prompt(format!("API key cho {provider}"))
                    .interact()?
            } else {
                anyhow::bail!(
                    "Thiếu API key cho {provider} và không có terminal để nhập ẩn. \
                     Dùng --api-key hoặc đặt biến môi trường (vd GROQ_API_KEY)."
                )
            }
        }
    };
    let api_key = api_key.trim().to_string();
    if let Some(url) = args.base_url.as_deref() {
        let url = url.trim();
        if url.starts_with("http://") {
            let host = url
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("");
            if !(host.starts_with("127.0.0.1")
                || host.starts_with("localhost")
                || host.starts_with("[::1]"))
            {
                eprintln!(
                    "[cảnh báo] base_url dùng HTTP không mã hóa: API key sẽ gửi dạng cleartext."
                );
            }
        }
    }
    if connection::requires_api_key(&provider) && api_key.is_empty() {
        anyhow::bail!(
            "API key rỗng cho {provider}. Nhập key thật, hoặc dùng --provider ollama/custom \
             nếu endpoint không cần key."
        );
    }

    // Cùng một loại provider thì cập nhật entry cũ thay vì thêm trùng.
    let existing_id = providers::load(&dir)?
        .providers
        .iter()
        .find(|entry| entry.provider == provider)
        .map(|entry| entry.id.clone());
    let saved = providers::upsert(
        &dir,
        ProviderInput {
            id: existing_id.as_deref(),
            name: "",
            provider: &provider,
            base_url: args.base_url.as_deref(),
            model: args.model.as_deref(),
            api_key: Some(&api_key),
            set_active: true,
        },
    )?;
    let vault = Vault::load_or_create(&dir)?;

    // Đảm bảo DB và profile tồn tại ngay từ đầu.
    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Người dùng")?;

    println!(
        "Đã kết nối. Sổ provider: {}",
        providers::path(&dir).display()
    );
    println!(
        "Provider: {} · model: {} · id: {}",
        saved.provider, saved.model, saved.id
    );
    println!(
        "Key được mã hóa bằng vault tại: {}",
        vault.key_path().display()
    );
    println!("Tiếp theo: tez chat --subject math \"Giải thích định lý Pythagore\"");
    Ok(())
}

async fn test_connection() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let saved = providers::active(&dir)?.ok_or_else(|| {
        anyhow::anyhow!("Chưa kết nối provider. Chạy: tez connect --provider groq")
    })?;
    let mut api_key = providers::decrypt_key(&dir, &saved)?;
    if api_key.trim().is_empty() {
        // Key lưu rỗng (provider cũ) nhưng env có key: dùng env để test.
        if let Some(key) = env_api_key(&saved.provider) {
            api_key = key;
        }
    }
    if api_key.trim().is_empty() && connection::requires_api_key(&saved.provider) {
        anyhow::bail!(
            "Chưa có API key cho '{}'. Chạy `tez connect --provider {}` \
             hoặc đặt biến môi trường tương ứng.",
            saved.provider,
            saved.provider
        );
    }
    let provider = OpenAiCompatible::new(&saved.provider, &saved.base_url, &api_key);

    let started = Instant::now();
    let reply =
        tez_llm::ping(&provider, &saved.model)
            .await
            .map_err(|error| match llm_error_hint(&error) {
                Some(hint) => anyhow::anyhow!("{hint}\nNguyên nhân: {error}"),
                None => anyhow::anyhow!(error),
            })?;
    println!(
        "OK · {} · {} · {}ms · {}",
        saved.provider,
        saved.model,
        started.elapsed().as_millis(),
        reply.trim()
    );
    Ok(())
}

fn provider_list() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let book = providers::load(&dir)?;
    if book.providers.is_empty() {
        println!("Chưa có provider nào. Chạy: tez connect --provider groq");
        return Ok(());
    }
    println!(
        "{:<26} {:<18} {:<12} {:<28} ĐANG DÙNG",
        "ID", "TÊN", "LOẠI", "MODEL"
    );
    for entry in &book.providers {
        let active = if book.active.as_deref() == Some(entry.id.as_str()) {
            "*"
        } else {
            ""
        };
        println!(
            "{:<26} {:<18} {:<12} {:<28} {}",
            entry.id, entry.name, entry.provider, entry.model, active
        );
    }
    Ok(())
}

fn provider_use(id: &str) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let entry = providers::set_active(&dir, id)?;
    println!(
        "Đang dùng: {} ({}) · {}",
        entry.name, entry.provider, entry.model
    );
    Ok(())
}

fn provider_remove(id: &str) -> anyhow::Result<()> {
    let dir = data_dir()?;
    if providers::remove(&dir, id)? {
        println!("Đã xóa provider {id}.");
        Ok(())
    } else {
        anyhow::bail!("Không tìm thấy provider {id}.")
    }
}

fn skills_list() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let registry = SkillRegistry::load_with_plugins(&dir)?;
    if registry.all().is_empty() {
        println!("Chưa có skill nào.");
        return Ok(());
    }
    println!("{:<24} {:<28} {:<20} NGUỒN", "ID", "TÊN", "MÔN");
    for skill in registry.all() {
        let subjects = if skill.subjects.is_empty() {
            "all".to_string()
        } else {
            skill.subjects.join(",")
        };
        println!(
            "{:<24} {:<28} {:<20} {}",
            skill.id, skill.name, subjects, skill.source
        );
    }
    Ok(())
}

fn skills_show(id: &str) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let registry = SkillRegistry::load_with_plugins(&dir)?;
    let skill = registry
        .find(id)
        .ok_or_else(|| anyhow::anyhow!("không tìm thấy skill '{id}'"))?;
    println!("{} ({})", skill.name, skill.id);
    println!("nguồn: {}", skill.source);
    if !skill.description.is_empty() {
        println!("mô tả: {}", skill.description);
    }
    println!("\n{}", skill.body);
    Ok(())
}

fn mcp_list() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let servers = mcp::load_servers(&dir)?;
    if servers.is_empty() {
        println!(
            "Chưa có MCP server. Tạo {} hoặc cài plugin.",
            mcp::config_path(&dir).display()
        );
        return Ok(());
    }
    for entry in servers {
        let status = if entry.config.enabled {
            "bật"
        } else {
            "tắt"
        };
        println!(
            "{:<16} {:<10} {:<8} {} {}",
            entry.name,
            entry.source,
            status,
            entry.config.command,
            entry.config.args.join(" ")
        );
    }
    Ok(())
}

async fn mcp_test(name: Option<String>) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let servers = mcp::load_servers(&dir)?;
    let selected: Vec<_> = servers
        .into_iter()
        .filter(|entry| entry.config.enabled)
        .filter(|entry| name.as_deref().is_none_or(|value| entry.name == value))
        .collect();
    if selected.is_empty() {
        println!("Không có MCP server phù hợp.");
        return Ok(());
    }
    for entry in selected {
        match mcp::McpClient::connect(&entry.name, &entry.config).await {
            Ok((_client, tools)) => {
                println!("{}: OK · {} tool", entry.name, tools.len());
                for tool in tools {
                    println!("  - {}: {}", tool.name, tool.description);
                }
            }
            Err(error) => println!("{}: LỖI · {error}", entry.name),
        }
    }
    Ok(())
}

fn plugins_list() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let plugins = plugins::load_all(&dir)?;
    if plugins.is_empty() {
        println!(
            "Chưa có plugin. Thư mục: {}",
            plugins::plugins_dir(&dir).display()
        );
        return Ok(());
    }
    for plugin in plugins {
        println!(
            "{} v{} · {} · {} skill, {} MCP server",
            plugin.manifest.name,
            plugin.manifest.version,
            plugin.manifest.description,
            plugin.manifest.skills.len(),
            plugin.manifest.mcp_servers.len()
        );
        println!("  {}", plugin.dir.display());
    }
    Ok(())
}

struct ResolvedConnection {
    provider_id: String,
    base_url: String,
    model: String,
    api_key: String,
}

fn resolve_chat_connection(
    args: &ChatArgs,
    book: &ProviderBook,
    dir: &Path,
) -> anyhow::Result<ResolvedConnection> {
    // Tên provider lạ không được âm thầm rơi về endpoint OpenAI mặc định.
    if let Some(requested) = args.provider.as_deref() {
        let saved = book.find(requested).is_some()
            || book
                .providers
                .iter()
                .any(|entry| entry.provider == requested);
        if !saved {
            connection::validate_provider(requested).map_err(|error| anyhow::anyhow!("{error}"))?;
        }
    }

    let selected = args
        .provider
        .as_deref()
        .and_then(|value| {
            book.find(value)
                .or_else(|| book.providers.iter().find(|entry| entry.provider == value))
        })
        .or_else(|| book.active());

    let provider_id = selected
        .map(|entry| entry.provider.clone())
        .or_else(|| args.provider.clone())
        .unwrap_or_else(|| "groq".to_string());
    let base_url = args
        .base_url
        .clone()
        .or_else(|| selected.map(|entry| entry.base_url.clone()))
        .unwrap_or_else(|| connection::default_base_url(&provider_id).to_string());
    let model = args
        .model
        .clone()
        .or_else(|| selected.map(|entry| entry.model.clone()))
        .unwrap_or_else(|| connection::default_model(&provider_id).to_string());

    let api_key = if let Some(key) = args.api_key.clone() {
        key
    } else if let Some(entry) = selected {
        providers::decrypt_key(dir, entry)?
    } else if let Some(key) = env_api_key(&provider_id) {
        key
    } else if provider_id == "ollama" {
        String::new()
    } else {
        anyhow::bail!(
            "Chưa có API key cho {provider_id}. Chạy: tez connect --provider {provider_id}\n\
             Hoặc đặt biến môi trường tương ứng và thử lại."
        );
    };

    Ok(ResolvedConnection {
        provider_id,
        base_url,
        model,
        api_key,
    })
}

/// Hỏi người dùng ngay trong terminal trước khi chạy tool rủi ro.
struct CliApprover;

#[async_trait]
impl Approver for CliApprover {
    async fn approve(&self, request: &ApprovalRequest) -> bool {
        let prompt = format!(
            "⚠ {} — công cụ: {} {}",
            request.reason, request.tool, request.args
        );
        tokio::task::spawn_blocking(move || {
            Confirm::new()
                .with_prompt(prompt)
                .default(false)
                .interact()
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }
}

struct TurnContext<'a> {
    dir: &'a Path,
    session_id: &'a str,
    pack: &'a SubjectPack,
    model: &'a str,
    provider_id: &'a str,
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    skills: SkillRegistry,
    mode: Mode,
    workspace: &'a Path,
    policy: Arc<PermissionPolicy>,
    approver: Arc<dyn Approver>,
    hooks: Arc<HookRegistry>,
    compaction: CompactSettings,
    memory: MemoryStore,
    memory_settings: harness::MemorySettings,
    todos: Arc<TodoState>,
    max_steps: usize,
    system_prompt: &'a str,
    language: &'a str,
    project_instructions: &'a str,
}

impl TurnContext<'_> {
    /// Chạy một lượt: RAG + context + agent + ghi DB. Trả về text cuối.
    async fn run_turn(
        &self,
        conversation: &mut Vec<Message>,
        prompt: &str,
    ) -> anyhow::Result<String> {
        let store = Store::open(self.dir.join("tez.db"))?;
        let subject_filter = if self.pack.is_general() {
            None
        } else {
            Some(self.pack.id)
        };
        let hits = store.search_chunks(prompt, subject_filter, 4)?;
        let excerpts: Vec<String> = hits
            .iter()
            .map(|hit| format!("[{}] {}", hit.title, hit.snippet))
            .collect();
        store.append_message(NewMessage {
            session_id: self.session_id,
            role: "user",
            content: prompt,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
        })?;
        drop(store);

        // Giữ tối đa 40 message gần nhất; compaction của agent xử lý phần còn lại.
        if conversation.len() > 40 {
            let drain = conversation.len() - 40;
            conversation.drain(0..drain);
        }

        let mut recent = conversation.clone();
        recent.push(Message::user(prompt));

        let skill_catalog = self.skills.catalog_lines(50);

        let recalled = if self.memory_settings.enabled && self.memory_settings.auto_recall {
            let records = self
                .memory
                .search(prompt, self.memory_settings.recall_limit);
            memory::render_recalled(&records)
        } else {
            Vec::new()
        };
        let todos = self.todos.snapshot();

        let workspace_label = self.workspace.display().to_string();
        let messages = ContextBuilder.build(ContextInput {
            excerpts: &excerpts,
            recent: &recent,
            skills: &skill_catalog,
            memory: &recalled,
            todos: &todos,
            mode: self.mode,
            workspace: Some(&workspace_label),
            instructions: self.system_prompt,
            project: self.project_instructions,
            language: self.language,
        });

        let artifact_dir = session_artifact_dir(self.dir, self.session_id);
        let compactor = Compactor::new(self.compaction.clone(), Some(artifact_dir.clone()));
        let agent = Agent::new(Arc::clone(&self.provider), self.tools.clone())
            .with_config(AgentConfig {
                max_steps: self.max_steps,
                ..AgentConfig::default()
            })
            .with_policy(Arc::clone(&self.policy))
            .with_approver(Arc::clone(&self.approver))
            .with_hooks(Arc::clone(&self.hooks))
            .with_compactor(compactor)
            .with_todos(Arc::clone(&self.todos));
        let started = Instant::now();
        let outcome = agent
            .run_with(
                self.model,
                messages,
                RunContext::new(self.session_id, Some(&artifact_dir), prompt),
            )
            .await?;
        let latency_ms = started.elapsed().as_millis() as u64;

        println!("{}", outcome.text);
        for step in &outcome.steps {
            let status = if step.ok { "ok" } else { "error" };
            eprintln!(
                "[tool] {} ({status}, {}ms) args={} -> {}",
                step.tool, step.duration_ms, step.args, step.output
            );
        }
        if outcome.compactions.changed() {
            eprintln!(
                "[context] nén: {} kết quả lớn, {} tin cũ, {} kết quả micro, tóm tắt={} · ~{} ký tự",
                outcome.compactions.persisted,
                outcome.compactions.snipped,
                outcome.compactions.micro_compacted,
                outcome.compactions.summarized,
                outcome.compactions.estimated_chars
            );
        }
        if outcome.permission_denials > 0 {
            eprintln!(
                "[permission] {} thao tác bị chặn/từ chối",
                outcome.permission_denials
            );
        }

        let store = Store::open(self.dir.join("tez.db"))?;
        store.append_message(NewMessage {
            session_id: self.session_id,
            role: "assistant",
            content: &outcome.text,
            model: Some(self.model),
            tokens_in: outcome.usage.input_tokens,
            tokens_out: outcome.usage.output_tokens,
        })?;
        for step in &outcome.steps {
            store.record_tool_call(
                self.session_id,
                &step.tool,
                &step.args,
                &step.output,
                step.ok,
                step.duration_ms,
            )?;
        }
        if let Ok(items_json) = serde_json::to_string(&outcome.todos) {
            store.save_todos(self.session_id, &items_json)?;
        }
        store.record_usage(UsageRecord {
            profile_id: "default",
            connection_id: None,
            provider_id: self.provider_id,
            model_id: self.model,
            task: "chat",
            tokens_in: outcome.usage.input_tokens,
            tokens_out: outcome.usage.output_tokens,
            latency_ms: Some(latency_ms),
            status: "ok",
        })?;
        drop(store);

        conversation.push(Message::user(prompt));
        conversation.push(Message::assistant(outcome.text.clone()));
        Ok(outcome.text)
    }
}

/// Đọc toàn bộ stdin khi có pipe. Trả `None` nếu đang chạy ở terminal.
fn read_stdin_prompt() -> anyhow::Result<Option<String>> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    use std::io::Read;
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt)?;
    if prompt.trim().is_empty() {
        anyhow::bail!("Câu hỏi rỗng.");
    }
    Ok(Some(prompt.trim().to_string()))
}

async fn chat(args: ChatArgs) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let book = providers::load(&dir)?;
    let mut mode = Mode::try_parse(&args.mode)
        .ok_or_else(|| anyhow::anyhow!("chế độ '{}' không hợp lệ. Chọn: chat, code", args.mode))?;
    let mut pack = subjects::resolve_or_general(&args.subject);
    let workspace = match args.workspace.clone() {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let connection = resolve_chat_connection(&args, &book, &dir)?;

    // Kiểm tra câu hỏi trước khi tạo session để không sinh session rác.
    if let Some(prompt) = args.prompt.as_deref() {
        if prompt.trim().is_empty() {
            anyhow::bail!("Câu hỏi rỗng.");
        }
    }
    let stdin_prompt = if args.prompt.is_none() {
        read_stdin_prompt()?
    } else {
        None
    };

    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Người dùng")?;
    let (session_id, mut conversation, todo_json) = match args.session.as_deref() {
        Some(id) => {
            let row = store.get_session(id)?.ok_or_else(|| {
                anyhow::anyhow!("không tìm thấy session '{id}'. Xem: tez sessions")
            })?;
            store.resume_session(id)?;
            // Session đã lưu quyết định chế độ và môn học; cờ CLI chỉ dùng khi tạo mới.
            mode = Mode::parse(&row.mode);
            pack = subjects::resolve_or_general(row.subject.as_deref().unwrap_or("general"));
            let conversation = store
                .load_messages(id, args.history)?
                .into_iter()
                .filter_map(|message| match message.role.as_str() {
                    "user" => Some(Message::user(message.content)),
                    "assistant" => Some(Message::assistant(message.content)),
                    _ => None,
                })
                .collect();
            let todos = store.load_todos(id)?;
            (id.to_string(), conversation, todos)
        }
        None => (
            store.create_session_mode("default", None, Some(pack.id), mode.as_str())?,
            Vec::new(),
            None,
        ),
    };
    drop(store);

    let todos = Arc::new(TodoState::new());
    if let Some(json) = todo_json {
        if let Ok(items) = serde_json::from_str::<Vec<tez_core::todo::TodoItem>>(&json) {
            let _ = todos.replace(items);
        }
    }

    let mut config = harness::load(&dir);
    if let Some(permission) = args.permission.as_deref() {
        config.permission_mode = PermissionMode::parse(permission);
    }
    if let Some(max_steps) = args.max_steps {
        config.max_steps = max_steps.clamp(1, 100);
    }
    if args.no_memory {
        config.memory.auto_recall = false;
    }
    let policy_mode = if mode == Mode::Plan {
        PermissionMode::ReadOnly
    } else {
        config.permission_mode
    };
    let policy =
        Arc::new(PermissionPolicy::new(policy_mode).with_rules(config.permission_rules.clone()));
    let approver: Arc<dyn Approver> = if args.yes || policy_mode == PermissionMode::Trust {
        Arc::new(AutoApprover)
    } else {
        Arc::new(CliApprover)
    };
    let hooks = Arc::new(hooks::build_registry(Some(&dir), &config.hooks));
    let memory = MemoryStore::new(dir.join("memory"));

    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    ));

    let mut tools = registry_for_mode(pack, mode, &workspace)?;
    if mode != Mode::Plan {
        tools.register(MemorySave::new(memory.clone()));
    }
    tools.register(MemorySearch::new(memory.clone()));
    if mode != Mode::Plan {
        tools.register(MemoryForget::new(memory.clone()));
    }
    tools.register(TodoWrite::new(Arc::clone(&todos)));
    if mode == Mode::Code {
        let runtime = Arc::new(SubagentRuntime {
            provider: Arc::clone(&provider),
            model: connection.model.clone(),
            tools: tools.clone(),
            policy: Arc::clone(&policy),
            approver: Arc::clone(&approver),
            hooks: Arc::clone(&hooks),
            max_steps: config.max_steps,
            compaction: config.compaction.clone(),
        });
        tools.register(TaskTool::new(runtime));
    }
    if mode != Mode::Plan {
        let discovery = mcp::discover_with_project(&dir, Some(&workspace)).await?;
        let mcp_count = discovery.tools.len();
        for server in &discovery.servers {
            if !server.ok {
                eprintln!(
                    "[mcp] {}: {}",
                    server.name,
                    server.error.as_deref().unwrap_or("không khả dụng")
                );
            }
        }
        for tool in discovery.tools {
            tools.register(tool);
        }
        if mcp_count > 0 {
            eprintln!(
                "[mcp] {mcp_count} tool từ {} server",
                discovery.servers.len()
            );
        }
    }
    let mut skills = SkillRegistry::load_with_plugins(&dir)?;
    skills.load_project(&workspace)?;
    tools.register(SkillTool::new(skills.clone()));
    tools.register(WebFetchTool::new());
    let project_instructions = project::ProjectContext::load(&workspace).render();

    let turn = TurnContext {
        dir: &dir,
        session_id: &session_id,
        pack,
        model: &connection.model,
        provider_id: &connection.provider_id,
        provider,
        tools,
        skills,
        mode,
        workspace: &workspace,
        policy,
        approver,
        hooks,
        compaction: config.compaction.clone(),
        memory,
        memory_settings: config.memory.clone(),
        todos,
        max_steps: config.max_steps,
        system_prompt: &config.system_prompt,
        language: &config.language,
        project_instructions: &project_instructions,
    };

    let interactive = args.prompt.is_none() && stdin_prompt.is_none();
    let mut last_summary = String::new();

    if let Some(prompt) = args.prompt.as_deref().or(stdin_prompt.as_deref()) {
        match turn.run_turn(&mut conversation, prompt).await {
            Ok(text) => last_summary = text,
            Err(error) => {
                // Không để session rỗng hoặc kẹt 'active' khi lượt chat thất bại.
                Store::open(dir.join("tez.db"))?.abandon_session(&session_id)?;
                return Err(with_llm_hint(error));
            }
        }
    } else if interactive {
        match mode {
            Mode::Code => {
                println!("tez code — workspace {}", workspace.display());
                println!("session {session_id}");
            }
            Mode::Plan => {
                println!("tez plan — chỉ đọc, workspace {}", workspace.display());
                println!("session {session_id}");
            }
            Mode::Chat => {
                println!(
                    "{} (lớp {}) — session {}",
                    if pack.is_general() {
                        "Trợ lý tổng quát".to_string()
                    } else {
                        format!("Gia sư {}", pack.name)
                    },
                    args.grade,
                    session_id
                );
            }
        }
        println!("Gõ câu hỏi, :help để xem lệnh, :q để thoát.\n");
        loop {
            print!("tez> ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line)? == 0 {
                println!();
                break;
            }
            let input = line.trim();
            if input.is_empty() {
                continue;
            }
            match input {
                ":q" | ":quit" | ":exit" => break,
                ":help" => {
                    println!(":help     xem trợ giúp");
                    println!(":session  in id session hiện tại");
                    println!(":q        thoát");
                    continue;
                }
                ":session" => {
                    println!("{session_id}");
                    continue;
                }
                _ => {}
            }

            match turn.run_turn(&mut conversation, input).await {
                Ok(text) => last_summary = text,
                Err(error) => eprintln!("[lỗi] {}", with_llm_hint(error)),
            }
            println!();
        }
    }

    let store = Store::open(dir.join("tez.db"))?;
    let summary: String = last_summary.chars().take(200).collect();
    store.end_session(
        &session_id,
        if summary.is_empty() {
            None
        } else {
            Some(summary.as_str())
        },
    )?;

    eprintln!(
        "[session] {session_id} | {} | {} | {} | tiếp tục: tez chat --session {session_id}",
        mode.as_str(),
        pack.id,
        connection.model
    );
    Ok(())
}

async fn update(args: UpdateArgs) -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("tez {current} · đang kiểm tra GitHub Releases…");
    let info = match tez_update::check(current).await {
        Ok(info) => info,
        Err(tez_update::UpdateError::NotFound(repo)) => {
            println!("Repo '{repo}' chưa có bản phát hành nào.");
            println!("Nếu bạn dùng bản fork, đặt biến môi trường TEZ_GITHUB_REPO=owner/name.");
            println!("Hoặc tải thủ công tại https://github.com/{repo}/releases");
            return Ok(());
        }
        Err(error) => {
            anyhow::bail!(
                "Không kiểm tra được cập nhật: {error}\n\
                 Bạn vẫn có thể tải bản mới nhất thủ công từ trang Releases của repo."
            );
        }
    };

    if info.available {
        println!("Có bản mới: {} (hiện tại {})", info.latest, info.current);
        println!("Trang tải: {}", info.url);
        if !info.notes.trim().is_empty() {
            println!("\nGhi chú phát hành:\n{}", info.notes.trim());
        }
    } else {
        println!("Bạn đang dùng bản mới nhất ({}).", info.current);
    }

    if args.download && !args.check_only {
        if let Some(asset) = tez_update::pick_asset(&info, std::env::consts::OS) {
            let dir = data_dir()?.join("updates");
            println!("\nĐang tải {}…", asset.name);
            let path = tez_update::download(asset, &dir).await?;
            println!("Đã tải: {}", path.display());
            println!(
                "Mở file trên để cài đặt (Windows: chạy installer; Linux: cài gói tương ứng)."
            );
        } else {
            println!("Bản phát hành không có asset phù hợp; mở trang tải ở trên.");
        }
    }

    Ok(())
}

fn doctor() -> anyhow::Result<()> {
    let dir = data_dir()?;
    println!("tez {}", env!("CARGO_PKG_VERSION"));
    println!("data dir: {}", dir.display());

    let book = providers::load(&dir)?;
    match book.active() {
        Some(entry) => println!(
            "provider: {} ({}) · {} provider đã lưu · id {}",
            entry.provider,
            entry.model,
            book.providers.len(),
            entry.id
        ),
        None => println!("provider: chưa có (chạy `tez connect`)"),
    }

    let store = Store::open(dir.join("tez.db"))?;
    let usage = store.usage_summary()?;
    println!(
        "db: {} ({}, {} session, {} request, {} in / {} out token)",
        store.path().display(),
        human_size(store.size_bytes()?),
        store.session_count()?,
        usage.requests,
        usage.tokens_in,
        usage.tokens_out
    );
    println!(
        "documents: {} file, {}",
        store.document_count()?,
        human_size(store.document_bytes()? as u64)
    );

    let vault = Vault::load_or_create(&dir)?;
    println!("vault key: {}", vault.key_path().display());

    for key in [
        "GROQ_API_KEY",
        "OPENROUTER_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "TEZ_API_KEY",
    ] {
        println!(
            "env {key}: {}",
            if std::env::var(key).is_ok() {
                "set"
            } else {
                "not set"
            }
        );
    }

    println!("modes: chat (tổng quát) · code (workspace tools)");
    println!(
        "subjects: {}",
        subjects::ALL
            .iter()
            .map(|pack| format!("{} ({})", pack.id, pack.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("code tools: {}", modes::CODE_TOOLS.join(", "));
    println!("tools: {}", full_registry().names().join(", "));
    let harness_config = harness::load(&dir);
    println!(
        "harness: permission={} max_steps={} compaction={} ({} ký tự) memory={} hooks={}",
        harness_config.permission_mode.as_str(),
        harness_config.max_steps,
        if harness_config.compaction.enabled {
            "bật"
        } else {
            "tắt"
        },
        harness_config.compaction.context_char_limit,
        if harness_config.memory.enabled {
            "bật"
        } else {
            "tắt"
        },
        harness_config.hooks.len()
    );
    println!("harness config: {}", harness::config_path(&dir).display());
    let skills = SkillRegistry::load_with_plugins(&dir)?;
    println!(
        "skills: {} (thư mục {})",
        skills.all().len(),
        dir.join("skills").display()
    );
    let mcp_servers = mcp::load_servers(&dir)?;
    println!("mcp servers: {}", mcp_servers.len());
    let plugins = plugins::load_all(&dir)?;
    println!("plugins: {}", plugins.len());
    println!("disk: chạy scripts/disk-guard.sh check");
    Ok(())
}

fn harness_show() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let config = harness::load(&dir);
    println!("harness config: {}", harness::config_path(&dir).display());
    println!(
        "permission: {} — {}",
        config.permission_mode.as_str(),
        config.permission_mode.description()
    );
    println!("max steps: {}", config.max_steps);
    println!("max tokens: {}", config.max_tokens);
    println!(
        "compaction: {} · giới hạn {} ký tự · trigger {} tin · target {} tin",
        if config.compaction.enabled {
            "bật"
        } else {
            "tắt"
        },
        config.compaction.context_char_limit,
        config.compaction.trigger_messages,
        config.compaction.target_messages
    );
    println!(
        "memory: {} · auto recall {} · tối đa {} bản ghi",
        if config.memory.enabled {
            "bật"
        } else {
            "tắt"
        },
        if config.memory.auto_recall {
            "bật"
        } else {
            "tắt"
        },
        config.memory.recall_limit
    );
    println!("hooks: {}", config.hooks.len());
    for hook in &config.hooks {
        println!(
            "  - {} → {} ({}s, {})",
            hook.event,
            hook.command,
            hook.timeout_secs,
            if hook.enabled { "bật" } else { "tắt" }
        );
    }
    println!(
        "memory dir: {} ({} bản ghi)",
        dir.join("memory").display(),
        MemoryStore::new(dir.join("memory")).records().len()
    );
    println!("audit log: {}", dir.join("audit.jsonl").display());
    Ok(())
}

fn memory_list() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = MemoryStore::new(dir.join("memory"));
    let records = store.records();
    if records.is_empty() {
        println!("Chưa có bản ghi memory nào. Agent có thể lưu bằng tool memory_save.");
        return Ok(());
    }
    println!("{:<28} {:<10} MÔ TẢ", "TÊN", "LOẠI");
    for record in records {
        println!(
            "{:<28} {:<10} {}",
            record.name, record.kind, record.description
        );
    }
    Ok(())
}

fn memory_search(query: &str) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = MemoryStore::new(dir.join("memory"));
    let records = store.search(query, 5);
    if records.is_empty() {
        println!("(không tìm thấy memory liên quan)");
        return Ok(());
    }
    for record in records {
        println!(
            "## {} ({})\n{}\n",
            record.name,
            record.kind,
            record.preview(600)
        );
    }
    Ok(())
}

fn memory_forget(name: &str) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = MemoryStore::new(dir.join("memory"));
    if store.forget(name).map_err(anyhow::Error::msg)? {
        println!("đã xoá memory '{name}'");
    } else {
        println!("không tìm thấy memory '{name}'");
    }
    Ok(())
}

fn subject_list() {
    for pack in subjects::ALL {
        println!(
            "{:<10} {:<10} strategy={:<8} tools={}",
            pack.id,
            pack.name,
            pack.strategy,
            pack.tools.join(", ")
        );
    }
}

fn subject_show(id: &str) -> anyhow::Result<()> {
    let pack = resolve_subject(id)?;
    println!("{} ({})", pack.name, pack.id);
    println!("chiến lược: {}", pack.strategy);
    println!("alias: {}", pack.aliases.join(", "));
    println!("tool: {}", pack.tools.join(", "));
    println!("\n--- prompt ---\n{}", pack.system_prompt());
    Ok(())
}

fn sessions(args: SessionsArgs) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    let rows = store.list_sessions(args.limit)?;
    if rows.is_empty() {
        println!("Chưa có session nào. Chạy: tez chat \"...\"");
        return Ok(());
    }

    println!(
        "{:<38} {:<8} {:<10} {:<8} {:<6} BẮT ĐẦU / TÓM TẮT",
        "ID", "CHẾ ĐỘ", "MÔN", "TRẠNG THÁI", "TIN"
    );
    for row in rows {
        let summary = row.summary.unwrap_or_default();
        let summary: String = summary.chars().take(60).collect();
        println!(
            "{:<38} {:<8} {:<10} {:<8} {:<6} {}  {}",
            row.id,
            row.mode,
            row.subject.unwrap_or_else(|| "-".to_string()),
            row.state,
            row.messages,
            row.started_at,
            summary
        );
    }
    Ok(())
}

fn usage() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    let summary = store.usage_summary()?;
    println!("requests: {}", summary.requests);
    println!("tokens in: {}", summary.tokens_in);
    println!("tokens out: {}", summary.tokens_out);
    println!("tokens total: {}", summary.tokens_in + summary.tokens_out);
    Ok(())
}

fn doc_add(args: DocAddArgs) -> anyhow::Result<()> {
    let extension = args
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(extension.as_str(), "txt" | "md" | "markdown" | "text") {
        anyhow::bail!(
            "hiện chỉ hỗ trợ .txt/.md (PDF/DOCX sẽ thêm sau): {}",
            args.path.display()
        );
    }

    let content = std::fs::read_to_string(&args.path)
        .with_context(|| format!("không đọc được {}", args.path.display()))?;
    if content.trim().is_empty() {
        anyhow::bail!("file rỗng: {}", args.path.display());
    }

    let title = args.title.unwrap_or_else(|| {
        args.path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Tài liệu")
            .to_string()
    });
    let subject = match args.subject.as_deref() {
        Some(input) => Some(resolve_subject(input)?.id.to_string()),
        None => None,
    };

    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Người dùng")?;
    let source = args.path.to_string_lossy().to_string();
    let mime = if matches!(extension.as_str(), "md" | "markdown") {
        "text/markdown"
    } else {
        "text/plain"
    };
    let (id, chunks) = store.add_document(
        "default",
        subject.as_deref(),
        &title,
        Some(&source),
        Some(mime),
        &content,
    )?;

    println!("Đã thêm tài liệu: {id}");
    println!("  tiêu đề: {title}");
    println!("  môn: {}", subject.as_deref().unwrap_or("chung"));
    println!("  chunk: {chunks}");
    println!("  dung lượng: {}", human_size(content.len() as u64));
    Ok(())
}

fn doc_list(args: DocListArgs) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    let documents = store.list_documents(args.limit)?;
    if documents.is_empty() {
        println!("Chưa có tài liệu. Thêm bằng: tez doc add <file.md> --subject math");
        return Ok(());
    }

    println!(
        "{:<38} {:<10} {:>6} {:>8} TIÊU ĐỀ",
        "ID", "MÔN", "CHUNK", "SIZE"
    );
    for document in documents {
        println!(
            "{:<38} {:<10} {:>6} {:>8} {}",
            document.id,
            document.subject.unwrap_or_else(|| "chung".to_string()),
            document.chunks,
            human_size(document.bytes as u64),
            document.title
        );
    }
    Ok(())
}

fn doc_search(args: DocSearchArgs) -> anyhow::Result<()> {
    let subject = match args.subject.as_deref() {
        Some(input) => Some(resolve_subject(input)?.id),
        None => None,
    };
    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    let hits = store.search_chunks(&args.query, subject, args.limit)?;
    if hits.is_empty() {
        println!("Không tìm thấy đoạn nào khớp '{}'.", args.query);
        return Ok(());
    }

    for hit in hits {
        println!(
            "[{}] {}\n  {}\n  doc: {}",
            hit.subject.as_deref().unwrap_or("chung"),
            hit.title,
            hit.snippet,
            hit.document_id
        );
    }
    Ok(())
}

fn doc_remove(id: &str) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    if store.delete_document(id)? {
        println!("Đã xóa tài liệu {id}.");
    } else {
        println!("Không tìm thấy tài liệu {id}.");
    }
    Ok(())
}

fn tools_list(subject: Option<String>) -> anyhow::Result<()> {
    let registry = match subject {
        Some(id) => registry_for(resolve_subject(&id)?),
        None => full_registry(),
    };
    for spec in registry.specs() {
        println!("{:<18} {}", spec.name, spec.description);
    }
    Ok(())
}

async fn tools_test(name: String, args: String, subject: Option<String>) -> anyhow::Result<()> {
    let registry = match subject {
        Some(id) => registry_for(resolve_subject(&id)?),
        None => full_registry(),
    };
    let value: Value = serde_json::from_str(&args)
        .with_context(|| format!("args không phải JSON hợp lệ: {args}"))?;
    match registry.execute(&name, value).await {
        Ok(output) => {
            println!("{}", output.content);
            if output.is_error {
                std::process::exit(4);
            }
        }
        // Tool chạy nhưng input sai cũng là lỗi thực thi: cùng mã thoát 4.
        Err(error @ (ToolError::InvalidArgs(_) | ToolError::Execution(_))) => {
            eprintln!("{error}");
            std::process::exit(4);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Rust bỏ qua SIGPIPE mặc định. Trả lại hành vi Unix chuẩn để lệnh như
/// `tez doctor | head` thoát im lặng (mã 141) thay vì panic khi stdout đóng.
#[cfg(unix)]
fn restore_sigpipe() {
    // SAFETY: `signal` chỉ đặt handler mặc định cho SIGPIPE, không truy cập bộ nhớ.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe() {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    restore_sigpipe();
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => doctor(),
        Commands::Connect(args) => connect(args).await,
        Commands::Test => test_connection().await,
        Commands::Chat(args) => chat(args).await,
        Commands::Update(args) => update(args).await,
        Commands::Subject { command } => match command {
            SubjectCommand::List => {
                subject_list();
                Ok(())
            }
            SubjectCommand::Show { id } => subject_show(&id),
        },
        Commands::Sessions(args) => sessions(args),
        Commands::Usage => usage(),
        Commands::Doc { command } => match command {
            DocCommand::Add(args) => doc_add(args),
            DocCommand::List(args) => doc_list(args),
            DocCommand::Search(args) => doc_search(args),
            DocCommand::Remove { id } => doc_remove(&id),
        },
        Commands::Tools { command } => match command {
            ToolsCommand::List { subject } => tools_list(subject),
            ToolsCommand::Test {
                name,
                args,
                subject,
            } => tools_test(name, args, subject).await,
        },
        Commands::Provider { command } => match command {
            ProviderCommand::List => provider_list(),
            ProviderCommand::Use { id } => provider_use(&id),
            ProviderCommand::Remove { id } => provider_remove(&id),
        },
        Commands::Skills { command } => match command {
            SkillsCommand::List => skills_list(),
            SkillsCommand::Show { id } => skills_show(&id),
        },
        Commands::Mcp { command } => match command {
            McpCommand::List => mcp_list(),
            McpCommand::Test { name } => mcp_test(name).await,
        },
        Commands::Plugins => plugins_list(),
        Commands::Memory { command } => match command {
            MemoryCommand::List => memory_list(),
            MemoryCommand::Search { query } => memory_search(&query),
            MemoryCommand::Forget { name } => memory_forget(&name),
        },
        Commands::Harness => harness_show(),
    }
}
