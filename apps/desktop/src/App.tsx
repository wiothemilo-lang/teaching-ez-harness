import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type SubjectInfo = {
  id: string;
  name: string;
  strategy: string;
  tools: string[];
};

type ConnectionInfo = {
  provider: string;
  model: string;
  base_url: string;
};

type AppStatus = {
  connection: ConnectionInfo | null;
  subjects: SubjectInfo[];
  sessions: number;
  data_dir: string;
  version: string;
};

type ToolTrace = {
  tool: string;
  args: string;
  output: string;
  ok: boolean;
  duration_ms: number;
};

type ChatReply = {
  session_id: string;
  text: string;
  tools: ToolTrace[];
  input_tokens: number;
  output_tokens: number;
};

type ChatMessage = {
  role: "user" | "assistant";
  text: string;
  tools?: ToolTrace[];
};

type SessionInfo = {
  id: string;
  subject: string | null;
  state: string;
  messages: number;
  started_at: string;
  summary: string | null;
};

type SessionDetail = {
  id: string;
  messages: { role: string; content: string }[];
};

type ConnectionTest = {
  ok: boolean;
  message: string;
  latency_ms: number;
};

type ProviderPreset = {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  keyHint: string;
  note: string;
};

const PROVIDERS: ProviderPreset[] = [
  {
    id: "groq",
    name: "Groq (free tier)",
    baseUrl: "https://api.groq.com/openai/v1",
    model: "llama-3.3-70b-versatile",
    keyHint: "gsk_…",
    note: "Nhanh, có free tier. Lấy key tại console.groq.com/keys.",
  },
  {
    id: "gemini",
    name: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    model: "gemini-2.0-flash",
    keyHint: "AIza…",
    note: "Có free tier. Lấy key tại aistudio.google.com/apikey.",
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    model: "meta-llama/llama-3.3-70b-instruct",
    keyHint: "sk-or-…",
    note: "Nhiều model, có model free. Lấy key tại openrouter.ai/keys.",
  },
  {
    id: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    keyHint: "sk-…",
    note: "Trả phí theo token. Dùng gpt-4o-mini để tiết kiệm.",
  },
  {
    id: "ollama",
    name: "Ollama (local)",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "qwen3:4b",
    keyHint: "không cần key",
    note: "Chạy hoàn toàn trên máy. Cần Ollama đang chạy và đã pull model.",
  },
  {
    id: "custom",
    name: "Custom OpenAI-compatible",
    baseUrl: "http://127.0.0.1:1234/v1",
    model: "local-model",
    keyHint: "không cần key nếu server local",
    note: "LM Studio, vLLM, gateway nội bộ… cần Base URL.",
  },
];

const SUBJECT_ICONS: Record<string, string> = {
  math: "∑",
  physics: "⚛",
  chemistry: "⚗",
  english: "A",
};

const SUGGESTIONS: Record<string, string[]> = {
  math: [
    "Giải thích định lý Pythagore bằng ví dụ thực tế",
    "Vẽ đồ thị y = x^2 - 4x + 3",
    "Kiểm tra giúp em bước giải 2x + 3 = 11",
  ],
  physics: [
    "Đổi 54 km/h sang m/s",
    "Giải thích công thức động năng",
    "Vật rơi tự do từ 20 m, tính thời gian rơi",
  ],
  chemistry: [
    "Cân bằng phương trình Fe + O2 -> Fe2O3",
    "Tính khối lượng mol của H2SO4",
    "Giải thích tỉ lệ mol trong phản ứng",
  ],
  english: [
    "Sửa lỗi: i have a apple",
    "Đoạn văn này có dễ đọc không?",
    "Giải thích cách dùng a và an",
  ],
};

const isTauri = "__TAURI_INTERNALS__" in window;
const demoMode = import.meta.env.DEV && !isTauri;

let demoConnection: ConnectionInfo | null = {
  provider: "demo",
  model: "mock-tez",
  base_url: "local",
};

const DEMO_SESSIONS: SessionInfo[] = [
  {
    id: "demo-session-0001",
    subject: "math",
    state: "ended",
    messages: 4,
    started_at: "2026-11-13 14:02:00",
    summary: "Định lý Pythagore và ví dụ tam giác 3-4-5",
  },
  {
    id: "demo-session-0002",
    subject: "chemistry",
    state: "ended",
    messages: 2,
    started_at: "2026-11-12 09:15:00",
    summary: "Cân bằng Fe + O2",
  },
];

const DEMO_STATUS: AppStatus = {
  connection: demoConnection,
  subjects: [
    {
      id: "math",
      name: "Toán",
      strategy: "socratic",
      tools: ["calculate", "check_step", "plot_function"],
    },
    {
      id: "physics",
      name: "Vật lý",
      strategy: "socratic",
      tools: ["calculate", "convert_units", "physics_formula"],
    },
    {
      id: "chemistry",
      name: "Hóa học",
      strategy: "socratic",
      tools: ["calculate", "molar_mass", "balance_equation"],
    },
    {
      id: "english",
      name: "Tiếng Anh",
      strategy: "coach",
      tools: ["check_grammar", "readability"],
    },
  ],
  sessions: 2,
  data_dir: "~/.local/share/teaching-ez (demo)",
  version: "0.1.0-demo",
};

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!demoMode) {
    return invoke<T>(command, args);
  }

  await new Promise((resolve) => setTimeout(resolve, 450));

  if (command === "app_status") {
    return { ...DEMO_STATUS, connection: demoConnection } as T;
  }
  if (command === "list_sessions") {
    return DEMO_SESSIONS as T;
  }
  if (command === "load_session") {
    const request = (args?.request ?? {}) as { session_id?: string };
    return {
      id: request.session_id ?? "demo-session-0001",
      messages: [
        { role: "user", content: "Giải thích định lý Pythagore bằng ví dụ thực tế" },
        {
          role: "assistant",
          content:
            "(Demo UI) Mình sẽ hướng dẫn từng bước: chọn tam giác vuông có hai cạnh góc vuông 3 cm và 4 cm, rồi tính cạnh huyền bằng căn bậc hai của 3² + 4².",
        },
      ],
    } as T;
  }
  if (command === "save_connection") {
    const request = (args?.request ?? {}) as {
      provider?: string;
      model?: string;
      base_url?: string;
    };
    demoConnection = {
      provider: request.provider ?? "demo",
      model: request.model || "mock-tez",
      base_url: request.base_url || "local",
    };
    return demoConnection as T;
  }
  if (command === "test_connection") {
    return { ok: true, message: "OK (demo)", latency_ms: 12 } as T;
  }
  if (command === "chat") {
    const request = (args?.request ?? {}) as { message?: string };
    return {
      session_id: "demo-session-0001",
      text: `(Demo UI) Mình đã nhận câu hỏi: “${request.message ?? ""}”.\n\nTrong bản Tauri thật, câu trả lời đến từ provider bạn đã kết nối, kèm tool call thật và lưu vào SQLite.`,
      tools: [
        {
          tool: "calculate",
          args: '{"expression":"(3+5)*2"}',
          output: "16",
          ok: true,
          duration_ms: 1,
        },
      ],
      input_tokens: 120,
      output_tokens: 40,
    } as T;
  }

  throw new Error(`demo: unknown command ${command}`);
}

function presetFor(id: string): ProviderPreset {
  return PROVIDERS.find((item) => item.id === id) ?? PROVIDERS[0];
}

function formatWhen(value: string): string {
  const normalized = value.includes("T") ? value : `${value.replace(" ", "T")}Z`;
  const date = new Date(normalized);
  if (Number.isNaN(date.getTime())) return value;

  const now = new Date();
  if (date.toDateString() === now.toDateString()) {
    return date.toLocaleTimeString("vi-VN", { hour: "2-digit", minute: "2-digit" });
  }

  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) {
    return "Hôm qua";
  }

  return date.toLocaleDateString("vi-VN", { day: "2-digit", month: "2-digit" });
}

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [subject, setSubject] = useState("math");
  const [grade, setGrade] = useState("10");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastUsage, setLastUsage] = useState<{ input: number; output: number } | null>(null);

  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [loadingSession, setLoadingSession] = useState(false);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [provider, setProvider] = useState("groq");
  const [model, setModel] = useState(presetFor("groq").model);
  const [baseUrl, setBaseUrl] = useState(presetFor("groq").baseUrl);
  const [apiKey, setApiKey] = useState("");
  const [saving, setSaving] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ConnectionTest | null>(null);

  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    call<AppStatus>("app_status")
      .then(setStatus)
      .catch((reason) => setError(String(reason)));
    void refreshSessions();
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, busy]);

  const currentSubject = status?.subjects.find((item) => item.id === subject);
  const suggestions = SUGGESTIONS[subject] ?? [];
  const selectedPreset = presetFor(provider);

  async function refreshSessions() {
    try {
      const rows = await call<SessionInfo[]>("list_sessions", { limit: 40 });
      setSessions(rows);
    } catch {
      // Lịch sử không quan trọng bằng việc chat; bỏ qua lỗi đọc danh sách.
    }
  }

  function subjectName(id: string | null | undefined): string {
    if (!id) return "Khác";
    return status?.subjects.find((item) => item.id === id)?.name ?? id;
  }

  async function send(textOverride?: string) {
    const text = (textOverride ?? input).trim();
    if (!text || busy) return;

    setInput("");
    setError(null);
    setBusy(true);
    setMessages((previous) => [...previous, { role: "user", text }]);

    try {
      const reply = await call<ChatReply>("chat", {
        request: {
          subject,
          grade,
          message: text,
          session_id: sessionId,
        },
      });
      setSessionId(reply.session_id);
      setLastUsage({ input: reply.input_tokens, output: reply.output_tokens });
      setMessages((previous) => [
        ...previous,
        { role: "assistant", text: reply.text, tools: reply.tools },
      ]);
      void refreshSessions();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function newSession() {
    setSessionId(null);
    setMessages([]);
    setLastUsage(null);
    setError(null);
  }

  function changeSubject(id: string) {
    if (id === subject) return;
    setSubject(id);
    newSession();
  }

  async function openSession(session: SessionInfo) {
    if (busy || loadingSession) return;
    setError(null);
    setLoadingSession(true);
    try {
      const detail = await call<SessionDetail>("load_session", {
        request: { session_id: session.id },
      });
      setSessionId(session.id);
      if (session.subject) setSubject(session.subject);
      setMessages(
        detail.messages
          .filter((message) => message.role === "user" || message.role === "assistant")
          .map((message) => ({
            role: message.role as "user" | "assistant",
            text: message.content,
          })),
      );
      setLastUsage(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoadingSession(false);
    }
  }

  function openSettings() {
    const current = status?.connection;
    const known = PROVIDERS.some((item) => item.id === current?.provider);
    const providerId = known && current ? current.provider : "groq";
    const preset = presetFor(providerId);
    setProvider(providerId);
    setModel(current?.model ?? preset.model);
    setBaseUrl(current?.base_url ?? preset.baseUrl);
    setApiKey("");
    setSettingsError(null);
    setTestResult(null);
    setSettingsOpen(true);
  }

  function changeProvider(id: string) {
    const preset = presetFor(id);
    setProvider(id);
    setModel(preset.model);
    setBaseUrl(preset.baseUrl);
    setSettingsError(null);
    setTestResult(null);
  }

  async function saveAndTest() {
    setSaving(true);
    setSettingsError(null);
    setTestResult(null);

    try {
      const info = await call<ConnectionInfo>("save_connection", {
        request: {
          provider,
          model,
          base_url: baseUrl,
          api_key: apiKey,
        },
      });
      setStatus((previous) => (previous ? { ...previous, connection: info } : previous));
      setApiKey("");

      try {
        const result = await call<ConnectionTest>("test_connection");
        setTestResult(result);
      } catch (reason) {
        setTestResult({ ok: false, message: String(reason), latency_ms: 0 });
      }
      void refreshSessions();
    } catch (reason) {
      setSettingsError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">tez</div>
          <div>
            <div className="brand-name">Gia sư AI</div>
            <div className="brand-sub">local-first</div>
          </div>
        </div>

        <div className="sidebar-label">Môn học</div>
        <nav className="subject-list">
          {(status?.subjects ?? []).map((item) => (
            <button
              key={item.id}
              className={`subject ${item.id === subject ? "subject-active" : ""}`}
              onClick={() => changeSubject(item.id)}
            >
              <span className="subject-icon">{SUBJECT_ICONS[item.id] ?? "•"}</span>
              <span className="subject-name">{item.name}</span>
              <span className="subject-tools">{item.tools.length}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar-label">Trình độ</div>
        <select className="grade" value={grade} onChange={(event) => setGrade(event.target.value)}>
          {["6", "7", "8", "9", "10", "11", "12"].map((value) => (
            <option key={value} value={value}>
              Lớp {value}
            </option>
          ))}
        </select>

        <div className="sidebar-label">Lịch sử</div>
        <div className="history">
          {sessions.length === 0 ? (
            <p className="history-empty">Chưa có phiên nào.</p>
          ) : (
            <div className="history-list">
              {sessions.map((item) => (
                <button
                  key={item.id}
                  className={`history-item ${item.id === sessionId ? "history-item-active" : ""}`}
                  onClick={() => void openSession(item)}
                  disabled={busy || loadingSession}
                  title={item.summary ?? item.id}
                >
                  <span className="history-top">
                    <span className="history-subject">{subjectName(item.subject)}</span>
                    <span className="history-time">{formatWhen(item.started_at)}</span>
                  </span>
                  <span className="history-meta">
                    {item.summary ?? `${item.messages} tin nhắn`}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="connection">
          <div className="sidebar-label">Provider</div>
          {status?.connection ? (
            <div className="connection-ok">
              <span className="dot" />
              <span>
                {status.connection.provider}
                <small>{status.connection.model}</small>
              </span>
            </div>
          ) : (
            <div className="connection-missing">
              <span>Chưa kết nối AI.</span>
              <span>Thêm API key hoặc dùng Ollama local.</span>
            </div>
          )}
          <button className="settings-button" onClick={openSettings}>
            Cài đặt kết nối
          </button>
        </div>
      </aside>

      <main className="main">
        <header className="topbar">
          <div>
            <h1>{currentSubject?.name ?? "Gia sư"}</h1>
            <p className="topbar-sub">
              {currentSubject
                ? `${currentSubject.tools.length} tool · ${currentSubject.strategy}`
                : "Đang tải…"}
              {sessionId ? ` · session ${sessionId.slice(0, 8)}` : ""}
            </p>
          </div>
          <div className="topbar-actions">
            {demoMode ? <span className="demo-badge">demo UI</span> : null}
            {lastUsage ? (
              <span className="usage">
                {lastUsage.input} in / {lastUsage.output} out
              </span>
            ) : null}
            <button className="ghost" onClick={openSettings}>
              Cài đặt
            </button>
            <button className="ghost" onClick={newSession} disabled={busy || loadingSession}>
              Session mới
            </button>
          </div>
        </header>

        <section className="chat">
          {messages.length === 0 ? (
            <div className="empty">
              <div className="empty-mark">{SUBJECT_ICONS[subject] ?? "∑"}</div>
              <h2>Học cùng tez</h2>
              <p>
                Mình sẽ hướng dẫn từng bước, dùng tool để tính toán và kiểm tra, thay vì đưa đáp án ngay.
              </p>

              {status && !status.connection ? (
                <div className="setup-hint">
                  <div>
                    <strong>Chưa kết nối AI.</strong>
                    <span>Groq có free tier, hoặc dùng Ollama ngay trên máy.</span>
                  </div>
                  <button onClick={openSettings}>Kết nối ngay</button>
                </div>
              ) : null}

              <div className="suggestions">
                {suggestions.map((suggestion) => (
                  <button key={suggestion} onClick={() => void send(suggestion)} disabled={busy}>
                    {suggestion}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            messages.map((message, index) => (
              <article key={index} className={`message message-${message.role}`}>
                <div className="avatar">{message.role === "user" ? "Em" : "tez"}</div>
                <div className="bubble">
                  <div className="bubble-text">{message.text}</div>
                  {message.tools && message.tools.length > 0 ? (
                    <div className="tools">
                      {message.tools.map((step, toolIndex) => (
                        <details key={toolIndex} className={`tool ${step.ok ? "" : "tool-error"}`}>
                          <summary>
                            <span className="tool-name">{step.tool}</span>
                            <span className="tool-meta">
                              {step.ok ? "ok" : "lỗi"} · {step.duration_ms}ms
                            </span>
                          </summary>
                          <pre>{step.output}</pre>
                        </details>
                      ))}
                    </div>
                  ) : null}
                </div>
              </article>
            ))
          )}

          {loadingSession ? (
            <article className="message message-assistant">
              <div className="avatar">tez</div>
              <div className="bubble typing">
                <span />
                <span />
                <span />
              </div>
            </article>
          ) : null}

          {busy ? (
            <article className="message message-assistant">
              <div className="avatar">tez</div>
              <div className="bubble typing">
                <span />
                <span />
                <span />
              </div>
            </article>
          ) : null}

          <div ref={bottomRef} />
        </section>

        {error ? (
          <div className="error" role="alert">
            <strong>Không gửi được.</strong>
            <span>{error}</span>
            <button onClick={() => setError(null)} aria-label="Đóng">
              ×
            </button>
          </div>
        ) : null}

        <footer className="composer">
          <textarea
            value={input}
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder={`Hỏi ${currentSubject?.name ?? "gia sư"}…`}
            rows={2}
            disabled={busy || loadingSession}
          />
          <button
            className="send"
            onClick={() => void send()}
            disabled={busy || loadingSession || !input.trim()}
          >
            Gửi
          </button>
        </footer>
        <div className="composer-hint">Enter để gửi · Shift+Enter xuống dòng · dữ liệu lưu trên máy bạn</div>
      </main>

      {settingsOpen ? (
        <div
          className="modal-backdrop"
          role="presentation"
          onClick={() => {
            if (!saving) setSettingsOpen(false);
          }}
        >
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="settings-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-head">
              <div>
                <h2 id="settings-title">Kết nối AI</h2>
                <p>API key được mã hóa AES-256-GCM và chỉ lưu trên máy bạn.</p>
              </div>
              <button
                className="icon-button"
                onClick={() => setSettingsOpen(false)}
                disabled={saving}
                aria-label="Đóng"
              >
                ×
              </button>
            </div>

            <div className="modal-body">
              <label className="field">
                <span>Provider</span>
                <select value={provider} onChange={(event) => changeProvider(event.target.value)}>
                  {PROVIDERS.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.name}
                    </option>
                  ))}
                </select>
                <small>{selectedPreset.note}</small>
              </label>

              <label className="field">
                <span>Model</span>
                <input
                  value={model}
                  onChange={(event) => setModel(event.target.value)}
                  placeholder={selectedPreset.model}
                />
              </label>

              {provider === "custom" || provider === "ollama" ? (
                <label className="field">
                  <span>Base URL</span>
                  <input
                    value={baseUrl}
                    onChange={(event) => setBaseUrl(event.target.value)}
                    placeholder={selectedPreset.baseUrl}
                  />
                </label>
              ) : null}

              <label className="field">
                <span>API key</span>
                <input
                  type="password"
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  placeholder={
                    status?.connection?.provider === provider
                      ? "Để trống để giữ key đã lưu"
                      : selectedPreset.keyHint
                  }
                  autoComplete="off"
                  spellCheck={false}
                />
              </label>

              {settingsError ? <div className="modal-error">{settingsError}</div> : null}

              {testResult ? (
                <div className={`test-result ${testResult.ok ? "test-ok" : "test-error"}`}>
                  <strong>{testResult.ok ? "Kết nối thành công" : "Chưa kiểm tra được"}</strong>
                  <span>
                    {testResult.ok
                      ? `${testResult.latency_ms}ms · ${testResult.message || "OK"}`
                      : testResult.message}
                  </span>
                </div>
              ) : null}
            </div>

            <div className="modal-actions">
              <span className="modal-path" title={status?.data_dir}>
                {status?.data_dir}
              </span>
              <button className="ghost" onClick={() => setSettingsOpen(false)} disabled={saving}>
                Đóng
              </button>
              <button className="send" onClick={() => void saveAndTest()} disabled={saving}>
                {saving ? "Đang kiểm tra…" : "Lưu & kiểm tra"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
