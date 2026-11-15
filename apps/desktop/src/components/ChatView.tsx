import { useEffect, useRef } from "react";
import type { KeyboardEvent } from "react";
import type {
  ChatMessage,
  CompactReport,
  ConnectionInfo,
  TodoItem,
  ToolTrace,
} from "../types";
import { CHAT_SUGGESTIONS, PLAN_SUGGESTIONS, placeholderFor } from "../lib";
import {
  IconArrowRight,
  IconArrowUp,
  IconChevronDown,
  IconKey,
  IconSliders,
  IconSparkle,
  IconStop,
  IconTerminal,
  IconX,
} from "../icons";
import { OkMark, Spinner } from "./ui";
import { HarnessDock } from "./HarnessDock";

type Props = {
  connection: ConnectionInfo | null;
  planMode: boolean;
  messages: ChatMessage[];
  input: string;
  busy: boolean;
  streaming: boolean;
  error: string | null;
  loadingSession: boolean;
  lastUsage: { input: number; output: number } | null;
  todos: TodoItem[];
  compaction: CompactReport | null;
  contextLimit: number;
  denials: number;
  onInput: (value: string) => void;
  onSend: () => void;
  onStop: () => void;
  onOpenSettings: () => void;
  onOpenHarness: () => void;
  onDismissError: () => void;
};

export function ChatView({
  connection,
  planMode,
  messages,
  input,
  busy,
  streaming,
  error,
  loadingSession,
  lastUsage,
  todos,
  compaction,
  contextLimit,
  denials,
  onInput,
  onSend,
  onStop,
  onOpenSettings,
  onOpenHarness,
  onDismissError,
}: Props) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const suggestions = planMode ? PLAN_SUGGESTIONS : CHAT_SUGGESTIONS;
  const placeholder = placeholderFor("general", planMode ? "plan" : "chat");

  useEffect(() => {
    const element = textareaRef.current;
    if (!element) return;
    element.style.height = "0px";
    element.style.height = `${Math.min(element.scrollHeight, 200)}px`;
  }, [input]);

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      if (input.trim() && !busy && !loadingSession) onSend();
    }
  };

  const pickSuggestion = (value: string) => {
    onInput(value);
    requestAnimationFrame(() => textareaRef.current?.focus());
  };

  return (
    <main className="chat">
      <header className="chat-header">
        <div className="chat-heading">
          <div className="chat-eyebrow">{planMode ? "READ-ONLY PLANNER" : "LOCAL AGENT"}</div>
          <h1 className="chat-title">{planMode ? "Plan" : "Chat"}</h1>
        </div>
        <div className="chat-header-actions">
          <button className="provider-chip" onClick={onOpenSettings} title="Quản lý nhà cung cấp">
            <span className={`status-dot ${connection ? "is-on" : ""}`} aria-hidden="true" />
            <span className="provider-chip-copy">
              <span className="provider-chip-label">Model</span>
              <span className="provider-chip-value">
                {connection ? `${connection.name} · ${connection.model}` : "Chưa kết nối"}
              </span>
            </span>
          </button>
          <button className="icon-button" onClick={onOpenSettings} aria-label="Mở cài đặt">
            <IconSliders size={18} />
          </button>
        </div>
      </header>

      <div className="chat-scroll">
        <div className="thread">
          {error && (
            <div className="error-banner" role="alert">
              <div className="error-copy">
                <strong>Không gửi được yêu cầu</strong>
                <p>{error}</p>
              </div>
              <div className="error-actions">
                <button className="btn btn-sm" onClick={onOpenSettings}>
                  Cài đặt
                </button>
                <button className="icon-button" onClick={onDismissError} aria-label="Đóng lỗi">
                  <IconX size={15} />
                </button>
              </div>
            </div>
          )}

          {loadingSession ? (
            <div className="thread-loading">
              <Spinner label="Đang mở phiên…" />
            </div>
          ) : messages.length === 0 ? (
            <section className="welcome">
              <div className="welcome-mark tone-general" aria-hidden="true">
                <IconSparkle size={22} />
              </div>
              <div className="welcome-eyebrow">{planMode ? "TEZ PLAN" : "TEZ AGENT"}</div>
              <h2 className="welcome-title">
                {planMode ? "Khảo sát trước, sửa sau." : "Bắt đầu từ bất cứ điều gì."}
              </h2>
              <p className="welcome-copy">
                {planMode
                  ? "Agent chỉ đọc workspace, đặt câu hỏi và trình kế hoạch. Khi duyệt xong, chuyển sang Code để thực thi."
                  : "Hỏi, viết, lập kế hoạch hoặc chuyển sang Code để đọc và sửa file. Mọi dữ liệu ở lại trên máy bạn."}
              </p>

              {planMode && (
                <div className="plan-note">
                  Chế độ Plan không ghi file và không chạy lệnh. Tool bị giới hạn còn đọc, tìm kiếm và
                  todo. Chuyển sang Code sẽ giữ nguyên hội thoại và kế hoạch.
                </div>
              )}

              {!connection && (
                <div className="connect-notice">
                  <span className="connect-icon">
                    <IconKey size={18} />
                  </span>
                  <div className="connect-copy">
                    <strong>Chưa kết nối nhà cung cấp.</strong>
                    <p>Thêm API key của Groq, Gemini, OpenRouter hoặc Ollama để bắt đầu.</p>
                  </div>
                  <button className="btn btn-primary btn-sm" onClick={onOpenSettings}>
                    Kết nối
                  </button>
                </div>
              )}

              <div className="suggestion-list">
                {suggestions.map((suggestion) => (
                  <button
                    key={suggestion}
                    className="suggestion"
                    onClick={() => pickSuggestion(suggestion)}
                  >
                    <span>{suggestion}</span>
                    <IconArrowRight size={16} />
                  </button>
                ))}
              </div>

              <p className="welcome-foot">SQLite trên máy · API key mã hóa · Không telemetry</p>
            </section>
          ) : (
            messages.map((message) => (
              <article key={message.id} className={`msg msg-${message.role}`}>
                <div className="msg-role">{message.role === "user" ? "Bạn" : "tez"}</div>
                <div className="msg-body">
                  <div className="msg-text">{message.text}</div>
                  {message.tools && message.tools.length > 0 && (
                    <ToolTraces tools={message.tools} />
                  )}
                </div>
              </article>
            ))
          )}

          {busy && !streaming && messages.length > 0 && (
            <div className="msg msg-assistant is-pending">
              <div className="msg-role">tez</div>
              <div className="msg-body">
                <div className="pending-line">
                  <span className="typing" aria-hidden="true">
                    <span />
                    <span />
                    <span />
                  </span>
                  Đang soạn câu trả lời…
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      <HarnessDock
        todos={todos}
        compaction={compaction}
        contextLimit={contextLimit}
        denials={denials}
        onOpenSettings={onOpenHarness}
      />

      <div className="composer-wrap">
        <div className="composer">
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(event) => onInput(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            rows={1}
            aria-label="Câu hỏi cho agent"
            disabled={loadingSession}
          />
          <div className="composer-foot">
            <div className="composer-hint">
              {busy ? (
                <span>Đang xử lý…</span>
              ) : (
                <>
                  <kbd>Enter</kbd> để gửi <span className="hint-sep">·</span>{" "}
                  <kbd>Shift + Enter</kbd> xuống dòng
                </>
              )}
            </div>
            <div className="composer-right">
              {lastUsage && (
                <span className="usage" title="Token của câu trả lời gần nhất">
                  {lastUsage.input} vào · {lastUsage.output} ra
                </span>
              )}
              <button
                className={`send ${busy ? "is-stop" : ""}`}
                onClick={busy ? onStop : onSend}
                disabled={!busy && (!input.trim() || loadingSession)}
                aria-label={busy ? "Dừng agent" : "Gửi câu hỏi"}
              >
                {busy ? <IconStop size={16} /> : <IconArrowUp size={18} />}
              </button>
            </div>
          </div>
        </div>
        <p className="composer-note">
          tez có thể mắc lỗi. Kiểm tra thông tin quan trọng trước khi dùng.
        </p>
      </div>
    </main>
  );
}

export function ToolTraces({ tools }: { tools: ToolTrace[] }) {
  const failed = tools.filter((tool) => !tool.ok).length;
  return (
    <details className="traces">
      <summary>
        <IconTerminal size={14} />
        <span>{tools.length} công cụ</span>
        {failed > 0 && <span className="trace-fail">{failed} lỗi</span>}
        <IconChevronDown size={14} className="trace-chevron" />
      </summary>
      <div className="trace-list">
        {tools.map((tool, index) => (
          <div className="trace" key={`${tool.tool}-${index}`}>
            <div className="trace-head">
              <code>{tool.tool}</code>
              <span className="trace-meta">
                <OkMark ok={tool.ok} />
                {tool.duration_ms} ms
              </span>
            </div>
            <div className="trace-args">
              <span>args</span>
              <code>{tool.args}</code>
            </div>
            <pre className="trace-output">{tool.output}</pre>
          </div>
        ))}
      </div>
    </details>
  );
}
