use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::Context as _;
use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use dialoguer::{Confirm, Password};
use directories::ProjectDirs;
use serde_json::Value;
use tez_auth::connection;
use tez_auth::providers::{self, ProviderBook, ProviderEntry, ProviderInput};
use tez_auth::Vault;
use tez_core::agent::{Agent, AgentConfig, RunContext};
use tez_core::compaction::{session_artifact_dir, CompactSettings, Compactor};
use tez_core::context::{ContextBuilder, ContextInput};
use tez_core::diff;
use tez_core::harness;
use tez_core::hooks::{self, HookRegistry};
use tez_core::mcp;
use tez_core::memory::{self, MemoryForget, MemorySave, MemorySearch, MemoryStore};
use tez_core::modes::{self, Mode, PLAN_TOOLS};
use tez_core::permission::{
    ApprovalRequest, Approver, AutoApprover, PermissionMode, PermissionPolicy,
};
use tez_core::plan;
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
use tez_core::tools::{code_registry, Tool, ToolError, ToolRegistry};
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
    #[arg(long)]
    mode: Option<String>,
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

fn data_dir_path() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "teaching-ez", "teaching-ez")
        .ok_or_else(|| anyhow::anyhow!("không xác định được thư mục dữ liệu"))?;
    Ok(dirs.data_dir().to_path_buf())
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dir = data_dir_path()?;
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

fn parse_mode(value: &str) -> anyhow::Result<Mode> {
    Mode::try_parse(value).ok_or_else(|| {
        anyhow::anyhow!(
            "chế độ '{}' không hợp lệ. Chọn: chat, code, plan",
            value.trim()
        )
    })
}

fn parse_permission_mode(value: &str) -> anyhow::Result<PermissionMode> {
    match value.trim().to_lowercase().as_str() {
        "readonly" | "read_only" | "read-only" => Ok(PermissionMode::ReadOnly),
        "strict" => Ok(PermissionMode::Strict),
        "standard" => Ok(PermissionMode::Standard),
        "trust" | "trusted" => Ok(PermissionMode::Trust),
        _ => anyhow::bail!(
            "permission '{}' không hợp lệ. Chọn: read_only, strict, standard, trust",
            value.trim()
        ),
    }
}

/// `LIMIT` của SQLite nhận số nguyên có dấu; chặn 0 và giá trị vượt i64.
fn validate_limit(limit: usize) -> anyhow::Result<usize> {
    if limit == 0 {
        anyhow::bail!("--limit phải lớn hơn 0.");
    }
    if i64::try_from(limit).is_err() {
        anyhow::bail!("--limit quá lớn (tối đa {}).", i64::MAX);
    }
    Ok(limit)
}

/// Bỏ mã ANSI/OSC và ký tự điều khiển để nội dung từ DB không phá terminal.
fn sanitize_terminal_text(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if used >= max_chars {
            break;
        }
        match ch {
            '\x1b' => match chars.next() {
                // OSC: ESC ] ... BEL hoặc ST (ESC \).
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // CSI: ESC [ tham số/intermediate ... byte kết thúc 0x40..=0x7e.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&next) {
                            break;
                        }
                    }
                }
                // Các chuỗi ESC khác: bỏ tới byte kết thúc 0x40..=0x7e.
                Some(next) if !('\x40'..='\x7e').contains(&next) => {
                    for next in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&next) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\r' | '\t' => {
                out.push(' ');
                used += 1;
            }
            ch if ch.is_control() => {}
            ch => {
                out.push(ch);
                used += 1;
            }
        }
    }
    out
}

async fn connect(args: ConnectArgs) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let provider = args.provider.trim().to_lowercase();
    connection::validate_provider(&provider)?;
    // `None` giữ key đã lưu; `Some("")` mới là chủ ý xóa key (custom/ollama).
    let api_key = match args.api_key {
        Some(key) => Some(key.trim().to_string()),
        None if !connection::requires_api_key(&provider) => None,
        None => {
            let key = if let Some(key) = env_api_key(&provider) {
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
            };
            Some(key.trim().to_string())
        }
    };
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
    if let Some(key) = api_key.as_deref() {
        if connection::requires_api_key(&provider) && key.is_empty() {
            anyhow::bail!(
                "API key rỗng cho {provider}. Nhập key thật, hoặc dùng --provider ollama/custom \
                 nếu endpoint không cần key."
            );
        }
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
            api_key: api_key.as_deref(),
            set_active: true,
        },
    )?;
    // Upsert đã bảo đảm vault tồn tại; chỉ mở key hiện có để không mint key mới.
    let vault = Vault::load(&dir)?;

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
    let selected: Vec<_> = if let Some(name) = name.as_deref() {
        let entry = servers
            .into_iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| anyhow::anyhow!("không tìm thấy MCP server '{name}'"))?;
        if !entry.config.enabled {
            anyhow::bail!(
                "MCP server '{name}' đang tắt. Bật trong {} rồi thử lại.",
                mcp::config_path(&dir).display()
            );
        }
        vec![entry]
    } else {
        servers
            .into_iter()
            .filter(|entry| entry.config.enabled)
            .collect()
    };
    if selected.is_empty() {
        println!("Không có MCP server phù hợp.");
        return Ok(());
    }
    let mut failures = 0usize;
    for entry in selected {
        match mcp::McpClient::connect(&entry.name, &entry.config).await {
            Ok((_client, tools)) => {
                println!("{}: OK · {} tool", entry.name, tools.len());
                for tool in tools {
                    println!("  - {}: {}", tool.name, tool.description);
                }
            }
            Err(error) => {
                failures += 1;
                eprintln!("{}: LỖI · {error}", entry.name);
            }
        }
    }
    if failures > 0 {
        anyhow::bail!("{failures} MCP server lỗi.");
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

struct ProviderChoice<'a> {
    entry: Option<&'a ProviderEntry>,
    kind: String,
}

fn find_saved_provider<'a>(book: &'a ProviderBook, requested: &str) -> Option<&'a ProviderEntry> {
    book.find(requested)
        .or_else(|| {
            book.providers
                .iter()
                .find(|entry| entry.id.eq_ignore_ascii_case(requested))
        })
        .or_else(|| {
            book.providers
                .iter()
                .find(|entry| entry.provider.eq_ignore_ascii_case(requested))
        })
}

/// Chọn provider theo `--provider`: id đã lưu trước, rồi tới loại đã biết.
///
/// `--provider` không bao giờ rơi về provider đang dùng: nếu không khớp id/loại
/// nào và không phải loại hỗ trợ sẵn thì báo lỗi ngay.
fn resolve_provider_choice<'a>(
    book: &'a ProviderBook,
    requested: Option<&str>,
) -> anyhow::Result<ProviderChoice<'a>> {
    match requested {
        Some(raw) => {
            let requested = raw.trim().to_lowercase();
            if requested.is_empty() {
                anyhow::bail!("--provider không được để trống.");
            }
            if let Some(entry) = find_saved_provider(book, &requested) {
                return Ok(ProviderChoice {
                    entry: Some(entry),
                    kind: entry.provider.to_lowercase(),
                });
            }
            connection::validate_provider(&requested)?;
            Ok(ProviderChoice {
                entry: None,
                kind: requested,
            })
        }
        None => Ok(match book.active() {
            Some(entry) => ProviderChoice {
                entry: Some(entry),
                kind: entry.provider.to_lowercase(),
            },
            None => ProviderChoice {
                entry: None,
                kind: "groq".to_string(),
            },
        }),
    }
}

/// Key hiệu lực: cờ CLI > key đã lưu > biến môi trường của provider.
///
/// Key đã lưu rỗng (provider cũ) vẫn được fallback sang env như `tez test`.
fn effective_api_key(
    explicit: Option<&str>,
    stored: Option<&str>,
    env: Option<String>,
    provider_id: &str,
) -> anyhow::Result<String> {
    if let Some(key) = explicit {
        let key = key.trim();
        if key.is_empty() && connection::requires_api_key(provider_id) {
            anyhow::bail!(
                "API key rỗng cho {provider_id}. Nhập key thật, hoặc dùng --provider ollama/custom \
                 nếu endpoint không cần key."
            );
        }
        return Ok(key.to_string());
    }
    if let Some(key) = stored.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(key.to_string());
    }
    if let Some(key) = env.map(|value| value.trim().to_string()) {
        if !key.is_empty() {
            return Ok(key);
        }
    }
    if connection::requires_api_key(provider_id) {
        anyhow::bail!(
            "Chưa có API key cho {provider_id}. Chạy: tez connect --provider {provider_id}\n\
             Hoặc đặt biến môi trường tương ứng và thử lại."
        );
    }
    Ok(String::new())
}

fn resolve_chat_connection(
    args: &ChatArgs,
    book: &ProviderBook,
    dir: &Path,
) -> anyhow::Result<ResolvedConnection> {
    let ProviderChoice { entry, kind } = resolve_provider_choice(book, args.provider.as_deref())?;
    let provider_id = kind;
    let base_url_override = args
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if provider_id == "custom" && entry.is_none() && base_url_override.is_none() {
        anyhow::bail!(
            "provider custom cần --base-url (hoặc chạy `tez connect --provider custom --base-url ...`)."
        );
    }
    let base_url = base_url_override
        .map(ToOwned::to_owned)
        .or_else(|| entry.map(|entry| entry.base_url.clone()))
        .unwrap_or_else(|| connection::default_base_url(&provider_id).to_string());
    connection::validate_base_url(&base_url)?;
    let model = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| entry.map(|entry| entry.model.clone()))
        .unwrap_or_else(|| connection::default_model(&provider_id).to_string());

    // Cờ --api-key thay thế hoàn toàn key đã lưu: không cần đọc vault.
    let stored_key = if args.api_key.is_some() {
        None
    } else {
        entry
            .map(|entry| providers::decrypt_key(dir, entry))
            .transpose()?
    };
    let api_key = effective_api_key(
        args.api_key.as_deref(),
        stored_key.as_deref(),
        env_api_key(&provider_id),
        &provider_id,
    )?;

    Ok(ResolvedConnection {
        provider_id,
        base_url,
        model,
        api_key,
    })
}

/// Hỏi người dùng ngay trong terminal trước khi chạy tool rủi ro.
struct CliApprover {
    workspace: PathBuf,
}

fn cli_approval_preview(root: &Path, request: &ApprovalRequest) -> Option<String> {
    if !matches!(request.tool.as_str(), "write_file" | "edit_file") {
        return None;
    }
    let raw = request.args.get("path")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        root.join(raw)
    };
    let root_canon = root.canonicalize().ok()?;
    let full = candidate.canonicalize().ok().or_else(|| {
        let parent = candidate.parent()?.canonicalize().ok()?;
        Some(parent.join(candidate.file_name()?))
    })?;
    if !full.starts_with(&root_canon) {
        return None;
    }

    let old = std::fs::read_to_string(&full).unwrap_or_default();
    let new = match request.tool.as_str() {
        "write_file" => request.args.get("content")?.as_str()?.to_string(),
        "edit_file" => {
            let old_string = request.args.get("old_string")?.as_str()?;
            let new_string = request.args.get("new_string")?.as_str()?;
            let replace_all = request
                .args
                .get("replace_all")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if old_string.is_empty() || !old.contains(old_string) {
                return None;
            }
            if !replace_all && old.matches(old_string).count() != 1 {
                return None;
            }
            if replace_all {
                old.replace(old_string, new_string)
            } else {
                old.replacen(old_string, new_string, 1)
            }
        }
        _ => return None,
    };

    let mut preview = diff::unified_diff(&old, &new, raw);
    if preview.chars().count() > 24_000 {
        preview = preview.chars().take(24_000).collect::<String>();
        preview.push_str("\n… diff đã rút gọn\n");
    }
    (!preview.is_empty()).then_some(preview)
}

#[async_trait]
impl Approver for CliApprover {
    async fn approve(&self, request: &ApprovalRequest) -> bool {
        // Stdout chỉ dành cho câu trả lời cuối; diff/preview phải ra stderr.
        if let Some(preview) = request
            .preview
            .clone()
            .or_else(|| cli_approval_preview(&self.workspace, request))
        {
            eprintln!("{preview}");
        }
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

        let skill_catalog = self.skills.catalog_overview(12);

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
        let saved_plan = if self.mode == Mode::Code {
            plan::load_plan(self.dir, self.session_id).ok().flatten()
        } else {
            None
        };
        let project_context = match saved_plan {
            Some(plan) => {
                let mut text = self.project_instructions.to_string();
                text.push_str("\nKế hoạch đã lưu của phiên:\n<plan>\n");
                text.extend(plan.chars().take(20_000));
                text.push_str("\n</plan>\n");
                text
            }
            None => self.project_instructions.to_string(),
        };
        let messages = ContextBuilder.build(ContextInput {
            excerpts: &excerpts,
            recent: &recent,
            skills: &skill_catalog,
            memory: &recalled,
            todos: &todos,
            mode: self.mode,
            workspace: Some(&workspace_label),
            instructions: self.system_prompt,
            project: &project_context,
            language: self.language,
        });

        let artifact_dir = session_artifact_dir(self.dir, self.session_id);
        let compactor = Compactor::new(self.compaction.clone(), Some(artifact_dir.clone()));
        let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let streamed = Arc::new(AtomicBool::new(false));
        let streamed_flag = Arc::clone(&streamed);
        let printer = tokio::task::spawn_blocking(move || {
            let mut stdout = std::io::stdout();
            while let Some(text) = delta_rx.blocking_recv() {
                if text.is_empty() {
                    continue;
                }
                streamed_flag.store(true, Ordering::Relaxed);
                let _ = stdout.write_all(text.as_bytes());
                let _ = stdout.flush();
            }
        });
        let agent = Agent::new(Arc::clone(&self.provider), self.tools.clone())
            .with_config(AgentConfig {
                max_steps: self.max_steps,
                ..AgentConfig::default()
            })
            .with_policy(Arc::clone(&self.policy))
            .with_approver(Arc::clone(&self.approver))
            .with_hooks(Arc::clone(&self.hooks))
            .with_compactor(compactor)
            .with_todos(Arc::clone(&self.todos))
            .with_stream(delta_tx);
        let started = Instant::now();
        let outcome = agent
            .run_with(
                self.model,
                messages,
                RunContext::new(self.session_id, Some(&artifact_dir), prompt),
            )
            .await?;
        let latency_ms = started.elapsed().as_millis() as u64;
        drop(agent);
        let _ = printer.await;

        if self.mode == Mode::Plan {
            if let Err(error) = plan::save_plan(self.dir, self.session_id, &outcome.text) {
                eprintln!("[plan] không lưu được kế hoạch: {error}");
            }
        }

        if !streamed.load(Ordering::Relaxed) {
            println!("{}", outcome.text);
        } else if !outcome.text.is_empty() {
            println!();
        }
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

struct ActiveSession {
    dir: PathBuf,
    session_id: String,
}

fn active_session_slot() -> &'static Mutex<Option<ActiveSession>> {
    static SLOT: OnceLock<Mutex<Option<ActiveSession>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn set_active_session(dir: &Path, session_id: &str) {
    if let Ok(mut slot) = active_session_slot().lock() {
        *slot = Some(ActiveSession {
            dir: dir.to_path_buf(),
            session_id: session_id.to_string(),
        });
    }
}

fn clear_active_session() {
    if let Ok(mut slot) = active_session_slot().lock() {
        *slot = None;
    }
}

struct ActiveSessionGuard {
    completed: bool,
}

impl ActiveSessionGuard {
    fn new() -> Self {
        Self { completed: false }
    }

    /// Session đã được đóng bình thường; không cần dọn ở `Drop`.
    fn finish(&mut self) {
        self.completed = true;
        clear_active_session();
    }
}

impl Drop for ActiveSessionGuard {
    fn drop(&mut self) {
        if self.completed {
            clear_active_session();
        } else {
            abandon_active_session();
        }
    }
}

/// Đánh dấu session đang mở là bỏ dở trước khi thoát vì tín hiệu.
fn abandon_active_session() {
    let active = active_session_slot()
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    let Some(active) = active else {
        return;
    };
    if let Ok(store) = Store::open(active.dir.join("tez.db")) {
        let _ = store.abandon_session(&active.session_id);
    }
}

fn shutdown_on_signal() -> ! {
    abandon_active_session();
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(130);
}

/// Ctrl-C/SIGTERM thoát mã 130 và không để session kẹt trạng thái `active`.
async fn install_signal_handler() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("[signal] không đăng ký được SIGINT: {error}");
                return;
            }
        };
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("[signal] không đăng ký được SIGTERM: {error}");
                return;
            }
        };
        tokio::spawn(async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
            shutdown_on_signal();
        });
    }
    #[cfg(not(unix))]
    {
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                shutdown_on_signal();
            }
        });
        tokio::task::yield_now().await;
    }
}

async fn chat(args: ChatArgs) -> anyhow::Result<()> {
    let requested_mode = args.mode.as_deref().map(parse_mode).transpose()?;
    let requested_permission = args
        .permission
        .as_deref()
        .map(parse_permission_mode)
        .transpose()?;
    let dir = data_dir()?;
    let book = providers::load(&dir)?;
    let mut mode = requested_mode.unwrap_or_default();
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
            // Môn học theo session đã lưu; --mode được phép ghi đè và lưu lại.
            let stored_mode = Mode::parse(&row.mode);
            mode = match requested_mode {
                Some(requested) => {
                    if stored_mode != requested {
                        store.update_session_mode(id, requested.as_str())?;
                    }
                    requested
                }
                None => stored_mode,
            };
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
    set_active_session(&dir, &session_id);
    let mut session_guard = ActiveSessionGuard::new();
    drop(store);

    let todos = Arc::new(TodoState::new());
    if let Some(json) = todo_json {
        if let Ok(items) = serde_json::from_str::<Vec<tez_core::todo::TodoItem>>(&json) {
            let _ = todos.replace(items);
        }
    }

    let mut config = harness::load(&dir);
    if let Some(permission) = requested_permission {
        config.permission_mode = permission;
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
    let approver: Arc<dyn Approver> = if args.yes || policy_mode == PermissionMode::Trust {
        Arc::new(AutoApprover)
    } else {
        Arc::new(CliApprover {
            workspace: workspace.clone(),
        })
    };
    let hooks = Arc::new(hooks::build_registry(Some(&dir), &config.hooks));
    let memory = MemoryStore::new(dir.join("memory"));

    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    ));

    let mut tools = registry_for_mode(pack, mode, &workspace)?;
    let mut read_only_tools = Vec::new();
    let discovery = mcp::discover_with_project(&dir, Some(&workspace)).await?;
    let mut mcp_count = 0;
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
        let read_only = tool.is_read_only();
        if mode == Mode::Plan && !read_only {
            continue;
        }
        if read_only {
            read_only_tools.push(tool.spec().name);
        }
        tools.register(tool);
        mcp_count += 1;
    }
    if mcp_count > 0 {
        eprintln!(
            "[mcp] {mcp_count} tool từ {} server",
            discovery.servers.len()
        );
    }

    let policy = Arc::new(
        PermissionPolicy::new(policy_mode)
            .with_rules(config.permission_rules.clone())
            .with_read_only_tools(read_only_tools),
    );
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
            // Guard dọn session rỗng hoặc đánh dấu 'failed' khi lượt chat lỗi.
            Err(error) => return Err(with_llm_hint(error)),
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
    session_guard.finish();

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
        if !info.available {
            println!("\nKhông có bản mới để tải.");
        } else if let Some(asset) = tez_update::pick_asset(&info, std::env::consts::OS) {
            let dir = data_dir()?.join("updates");
            println!("\nĐang tải {}…", asset.name);
            let path = tez_update::download(asset, &info.assets, &dir).await?;
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

/// Nạp skill đã có mà không seed thư mục mặc định (doctor phải chỉ đọc).
fn load_skills_read_only(dir: &Path) -> anyhow::Result<SkillRegistry> {
    let mut registry = SkillRegistry::default();
    registry.load_extra(&[dir.join("skills")], "user")?;
    for plugin in plugins::load_all(dir)? {
        registry.load_extra(&plugin.skill_paths(), &plugin.manifest.id)?;
    }
    Ok(registry)
}

fn doctor() -> anyhow::Result<()> {
    let dir = data_dir_path()?;
    println!("tez {}", env!("CARGO_PKG_VERSION"));
    println!("data dir: {}", dir.display());
    if !dir.exists() {
        println!("data dir: chưa tồn tại (doctor chỉ đọc, không tạo mới)");
    }

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

    let db_path = dir.join("tez.db");
    if db_path.exists() {
        match Store::open_read_only(&db_path) {
            Ok(store) => {
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
            }
            Err(error) => println!("db: {} (không đọc được: {error})", db_path.display()),
        }
    } else {
        println!(
            "db: chưa có ({}) — chạy `tez chat` để tạo",
            db_path.display()
        );
    }

    match Vault::load(&dir) {
        Ok(vault) => println!("vault key: {}", vault.key_path().display()),
        Err(error) => println!("vault: thiếu/lỗi master.key · {error}"),
    }

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

    println!("modes: chat (tổng quát) · plan (chỉ đọc) · code (workspace tools)");
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
    let skills = load_skills_read_only(&dir)?;
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
    let limit = validate_limit(args.limit)?;
    let dir = data_dir()?;
    let store = Store::open(dir.join("tez.db"))?;
    let rows = store.list_sessions(limit)?;
    if rows.is_empty() {
        println!("Chưa có session nào. Chạy: tez chat \"...\"");
        return Ok(());
    }

    println!(
        "{:<38} {:<8} {:<10} {:<8} {:<6} BẮT ĐẦU / TÓM TẮT",
        "ID", "CHẾ ĐỘ", "MÔN", "TRẠNG THÁI", "TIN"
    );
    for row in rows {
        let summary = sanitize_terminal_text(row.summary.as_deref().unwrap_or_default(), 60);
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
    install_signal_handler().await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_entry(id: &str, provider: &str) -> ProviderEntry {
        ProviderEntry {
            id: id.to_string(),
            name: provider.to_string(),
            provider: provider.to_string(),
            base_url: connection::default_base_url(provider).to_string(),
            model: connection::default_model(provider).to_string(),
            encrypted_key: "ciphertext".to_string(),
        }
    }

    fn sample_book() -> ProviderBook {
        ProviderBook {
            version: 1,
            active: Some("groq-1".to_string()),
            providers: vec![provider_entry("groq-1", "groq")],
        }
    }

    #[test]
    fn provider_flag_known_kind_does_not_fall_back_to_active() {
        let book = sample_book();
        let choice = resolve_provider_choice(&book, Some("custom")).unwrap();
        assert!(choice.entry.is_none());
        assert_eq!(choice.kind, "custom");
    }

    #[test]
    fn provider_flag_unknown_kind_errors() {
        let book = sample_book();
        assert!(resolve_provider_choice(&book, Some("nope")).is_err());
        assert!(resolve_provider_choice(&book, Some("  ")).is_err());
    }

    #[test]
    fn provider_flag_matches_saved_id_case_insensitively() {
        let book = sample_book();
        let choice = resolve_provider_choice(&book, Some("GROQ-1")).unwrap();
        assert_eq!(choice.entry.unwrap().id, "groq-1");
        assert_eq!(choice.kind, "groq");
    }

    #[test]
    fn provider_flag_matches_saved_kind() {
        let book = sample_book();
        let choice = resolve_provider_choice(&book, Some("groq")).unwrap();
        assert_eq!(choice.entry.unwrap().id, "groq-1");
    }

    #[test]
    fn provider_defaults_to_active_without_flag() {
        let book = sample_book();
        let choice = resolve_provider_choice(&book, None).unwrap();
        assert_eq!(choice.entry.unwrap().id, "groq-1");
    }

    #[test]
    fn empty_stored_key_falls_back_to_env() {
        let key = effective_api_key(None, Some(""), Some("env-key".to_string()), "groq").unwrap();
        assert_eq!(key, "env-key");
    }

    #[test]
    fn stored_key_wins_over_env() {
        let key = effective_api_key(
            None,
            Some("stored-key"),
            Some("env-key".to_string()),
            "groq",
        )
        .unwrap();
        assert_eq!(key, "stored-key");
    }

    #[test]
    fn explicit_key_wins_over_stored_key() {
        let key = effective_api_key(
            Some("explicit-key"),
            Some("stored-key"),
            Some("env-key".to_string()),
            "groq",
        )
        .unwrap();
        assert_eq!(key, "explicit-key");
    }

    #[test]
    fn cloud_provider_without_key_errors() {
        assert!(effective_api_key(None, None, None, "groq").is_err());
        assert!(effective_api_key(Some(""), None, None, "openai").is_err());
    }

    #[test]
    fn keyless_provider_without_key_is_allowed() {
        assert_eq!(effective_api_key(None, None, None, "custom").unwrap(), "");
        assert_eq!(effective_api_key(None, None, None, "ollama").unwrap(), "");
    }

    #[test]
    fn parses_modes() {
        assert_eq!(parse_mode("chat").unwrap(), Mode::Chat);
        assert_eq!(parse_mode("PLAN").unwrap(), Mode::Plan);
        assert_eq!(parse_mode("code").unwrap(), Mode::Code);
    }

    #[test]
    fn invalid_mode_lists_choices() {
        let error = parse_mode("bogus").unwrap_err().to_string();
        assert!(error.contains("chat"), "{error}");
        assert!(error.contains("code"), "{error}");
        assert!(error.contains("plan"), "{error}");
    }

    #[test]
    fn parses_permission_modes() {
        assert_eq!(
            parse_permission_mode("read-only").unwrap(),
            PermissionMode::ReadOnly
        );
        assert_eq!(
            parse_permission_mode("STRICT").unwrap(),
            PermissionMode::Strict
        );
        assert_eq!(
            parse_permission_mode("standard").unwrap(),
            PermissionMode::Standard
        );
        assert_eq!(
            parse_permission_mode("trusted").unwrap(),
            PermissionMode::Trust
        );
        assert!(parse_permission_mode("bogus").is_err());
    }

    #[test]
    fn rejects_zero_limit() {
        assert!(validate_limit(0).is_err());
    }

    #[test]
    fn accepts_positive_limit() {
        assert_eq!(validate_limit(20).unwrap(), 20);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn rejects_limit_above_i64_max() {
        let max = usize::try_from(i64::MAX).unwrap();
        assert!(validate_limit(max).is_ok());
        assert!(validate_limit(max + 1).is_err());
    }

    #[test]
    fn strips_ansi_sequences() {
        assert_eq!(sanitize_terminal_text("\x1b[31mred\x1b[0m", 60), "red");
        assert_eq!(
            sanitize_terminal_text("\x1b[2K\x1b]0;title\x07ok", 60),
            "ok"
        );
    }

    #[test]
    fn replaces_control_chars_and_truncates() {
        assert_eq!(sanitize_terminal_text("a\nb\tc\x07d", 60), "a b cd");
        assert_eq!(sanitize_terminal_text("\x1b[31mabcdef\x1b[0m", 3), "abc");
    }
}
