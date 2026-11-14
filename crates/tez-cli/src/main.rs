use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use dialoguer::Password;
use directories::ProjectDirs;
use serde_json::Value;
use tez_auth::connection::{self, ConnectionFile};
use tez_auth::Vault;
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
    /// Chat với tutor agent.
    Chat(ChatArgs),
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
    /// Môn học: math, physics, chemistry, english (hoặc Toán, Lý, Hóa, Anh).
    #[arg(long, default_value = "math")]
    subject: String,
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
    /// Câu hỏi; bỏ trống để vào chế độ chat tương tác (hoặc đọc stdin nếu có pipe).
    prompt: Option<String>,
}

#[derive(Args)]
struct SessionsArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
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
        .filter(|value| !value.trim().is_empty())
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
    let api_key = match args.api_key {
        Some(key) => key,
        None if args.provider == "ollama" => String::new(),
        None => Password::new()
            .with_prompt(format!("API key cho {}", args.provider))
            .interact()?,
    };

    let saved = connection::save(
        &dir,
        &args.provider,
        args.base_url.as_deref(),
        args.model.as_deref(),
        &api_key,
    )?;
    let vault = Vault::load_or_create(&dir)?;

    // Đảm bảo DB và profile tồn tại ngay từ đầu.
    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Học sinh")?;

    println!("Đã kết nối. Cấu hình: {}", connection::path(&dir).display());
    println!("Provider: {} · model: {}", saved.provider, saved.model);
    println!(
        "Key được mã hóa bằng vault tại: {}",
        vault.key_path().display()
    );
    println!("Tiếp theo: tez chat --subject math \"Giải thích định lý Pythagore\"");
    Ok(())
}

async fn test_connection() -> anyhow::Result<()> {
    let dir = data_dir()?;
    let saved = connection::load(&dir)?.ok_or_else(|| {
        anyhow::anyhow!("Chưa kết nối provider. Chạy: tez connect --provider groq")
    })?;
    let api_key = connection::decrypt_key(&dir, &saved)?;
    let provider = OpenAiCompatible::new(&saved.provider, &saved.base_url, &api_key);

    let started = Instant::now();
    let reply = tez_llm::ping(&provider, &saved.model).await?;
    println!(
        "OK · {} · {} · {}ms · {}",
        saved.provider,
        saved.model,
        started.elapsed().as_millis(),
        reply.trim()
    );
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
    saved: Option<&ConnectionFile>,
    dir: &Path,
) -> anyhow::Result<ResolvedConnection> {
    let provider_id = args
        .provider
        .clone()
        .or_else(|| saved.map(|c| c.provider.clone()))
        .unwrap_or_else(|| "groq".to_string());
    let saved_for_provider = saved.filter(|c| c.provider == provider_id);

    let base_url = args
        .base_url
        .clone()
        .or_else(|| saved_for_provider.map(|c| c.base_url.clone()))
        .unwrap_or_else(|| connection::default_base_url(&provider_id).to_string());
    let model = args
        .model
        .clone()
        .or_else(|| saved_for_provider.map(|c| c.model.clone()))
        .unwrap_or_else(|| connection::default_model(&provider_id).to_string());

    let api_key = if let Some(key) = args.api_key.clone() {
        key
    } else if let Some(saved) = saved_for_provider {
        connection::decrypt_key(dir, saved)?
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

struct TurnContext<'a> {
    dir: &'a Path,
    session_id: &'a str,
    pack: &'a SubjectPack,
    grade: &'a str,
    model: &'a str,
    provider_id: &'a str,
    provider: Arc<dyn LlmProvider>,
}

impl TurnContext<'_> {
    /// Chạy một lượt: RAG + context + agent + ghi DB. Trả về text cuối.
    async fn run_turn(
        &self,
        conversation: &mut Vec<Message>,
        prompt: &str,
    ) -> anyhow::Result<String> {
        let store = Store::open(self.dir.join("tez.db"))?;
        let hits = store.search_chunks(prompt, Some(self.pack.id), 4)?;
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

        // Giữ tối đa 20 message gần nhất để context không phình vô hạn.
        if conversation.len() > 20 {
            let drain = conversation.len() - 20;
            conversation.drain(0..drain);
        }

        let mut recent = conversation.clone();
        recent.push(Message::user(prompt));

        let objectives: Vec<String> = Vec::new();
        let messages = ContextBuilder.build(ContextInput {
            subject: self.pack,
            grade: self.grade,
            objectives: &objectives,
            excerpts: &excerpts,
            student_summary: None,
            state: "practice",
            recent: &recent,
        });

        let agent = Agent::new(Arc::clone(&self.provider), registry_for(self.pack));
        let started = Instant::now();
        let outcome = agent.run(self.model, messages).await?;
        let latency_ms = started.elapsed().as_millis() as u64;

        println!("{}", outcome.text);
        for step in &outcome.steps {
            let status = if step.ok { "ok" } else { "error" };
            eprintln!(
                "[tool] {} ({status}, {}ms) args={} -> {}",
                step.tool, step.duration_ms, step.args, step.output
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

async fn chat(args: ChatArgs) -> anyhow::Result<()> {
    let dir = data_dir()?;
    let saved = connection::load(&dir)?;
    let pack = resolve_subject(&args.subject)?;
    let connection = resolve_chat_connection(&args, saved.as_ref(), &dir)?;

    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Học sinh")?;
    let (session_id, mut conversation) = match args.session.as_deref() {
        Some(id) => {
            if !store.session_exists(id)? {
                anyhow::bail!("không tìm thấy session '{id}'. Xem: tez sessions");
            }
            store.resume_session(id)?;
            let conversation = store
                .load_messages(id, args.history)?
                .into_iter()
                .filter_map(|message| match message.role.as_str() {
                    "user" => Some(Message::user(message.content)),
                    "assistant" => Some(Message::assistant(message.content)),
                    _ => None,
                })
                .collect();
            (id.to_string(), conversation)
        }
        None => (
            store.create_session("default", None, Some(pack.id))?,
            Vec::new(),
        ),
    };
    drop(store);

    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    ));
    let turn = TurnContext {
        dir: &dir,
        session_id: &session_id,
        pack,
        grade: &args.grade,
        model: &connection.model,
        provider_id: &connection.provider_id,
        provider,
    };

    let interactive = args.prompt.is_none() && std::io::stdin().is_terminal();
    let mut last_summary = String::new();

    if let Some(prompt) = args.prompt.as_deref() {
        if prompt.trim().is_empty() {
            anyhow::bail!("Câu hỏi rỗng.");
        }
        last_summary = turn.run_turn(&mut conversation, prompt).await?;
    } else if interactive {
        println!(
            "Gia sư {} (lớp {}) — session {}",
            pack.name, args.grade, session_id
        );
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
                Err(error) => eprintln!("[lỗi] {error}"),
            }
            println!();
        }
    } else {
        use std::io::Read;
        let mut prompt = String::new();
        std::io::stdin().read_to_string(&mut prompt)?;
        if prompt.trim().is_empty() {
            anyhow::bail!("Câu hỏi rỗng.");
        }
        last_summary = turn.run_turn(&mut conversation, prompt.trim()).await?;
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
        "[session] {session_id} | {} | {} | chat tiếp: tez chat --session {session_id}",
        pack.id, connection.model
    );
    Ok(())
}

fn doctor() -> anyhow::Result<()> {
    let dir = data_dir()?;
    println!("tez {}", env!("CARGO_PKG_VERSION"));
    println!("data dir: {}", dir.display());

    let connection = connection::load(&dir)?;
    match &connection {
        Some(c) => println!("connection: {} ({})", c.provider, c.model),
        None => println!("connection: chưa có (chạy `tez connect`)"),
    }

    let store = Store::open(dir.join("tez.db"))?;
    let usage = store.usage_summary()?;
    println!(
        "db: {} ({}, {} session, {} request, {} in / {} out token)",
        store.path().display(),
        human_size(store.size_bytes()),
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

    println!(
        "subjects: {}",
        subjects::ALL
            .iter()
            .map(|pack| format!("{} ({})", pack.id, pack.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("tools: {}", full_registry().names().join(", "));
    println!("disk: chạy scripts/disk-guard.sh check");
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
        "{:<38} {:<10} {:<8} {:<6} BẮT ĐẦU / TÓM TẮT",
        "ID", "MÔN", "TRẠNG THÁI", "TIN"
    );
    for row in rows {
        let summary = row.summary.unwrap_or_default();
        let summary: String = summary.chars().take(60).collect();
        println!(
            "{:<38} {:<10} {:<8} {:<6} {}  {}",
            row.id,
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
    store.ensure_profile("default", "Học sinh")?;
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
    let output = registry.execute(&name, value).await?;
    println!("{}", output.content);
    if output.is_error {
        std::process::exit(4);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => doctor(),
        Commands::Connect(args) => connect(args).await,
        Commands::Test => test_connection().await,
        Commands::Chat(args) => chat(args).await,
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
    }
}
