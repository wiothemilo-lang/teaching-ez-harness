import { useEffect, useRef } from "react";
import type { KeyboardEvent } from "react";
import type { ChatMessage, CompactReport, ConnectionInfo, TodoItem } from "../types";
import { CODE_SUGGESTIONS, placeholderFor } from "../lib";
import {
  IconArrowRight,
  IconArrowUp,
  IconCode,
  IconFolder,
  IconKey,
  IconSliders,
  IconTerminal,
  IconX,
} from "../icons";
import { ToolTraces } from "./ChatView";
import { HarnessDock } from "./HarnessDock";
import { Spinner } from "./ui";

type Props = {
  connection: ConnectionInfo | null;
  messages: ChatMessage[];
  input: string;
  busy: boolean;
  error: string | null;
  loadingSession: boolean;
  lastUsage: { input: number; output: number } | null;
  todos: TodoItem[];
  compaction: CompactReport | null;
  contextLimit: number;
  denials: number;
  workspace: string;
  onWorkspace: (value: string) => void;
  onOpenWorkspace: () => void;
  onInput: (value: string) => void;
  onSend: () => void;
  onOpenSettings: () => void;
  onOpenHarness: () => void;
  onDismissError: () => void;
};

export function CodeView({
  connection,
  messages,
  input,
  busy,
  error,
  loadingSession,
  lastUsage,
  todos,
  compaction,
  contextLimit,
  denials,
  workspace,
  onWorkspace,
  onOpenWorkspace,
  onInput,
  onSend,
  onOpenSettings,
  onOpenHarness,
  onDismissError,
}: Props) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const element = textareaRef.current;
    if (!element) return;
    element.style.height = "0px";
    element.style.height = `${Math.min(element.scrollHeight, 220)}px`;
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
    <main className="chat code-shell">
      <header className="chat-header code-header">
        <div className="chat-heading">
          <div className="chat-eyebrow">CODE · LOCAL WORKSPACE</div>
          <h1 className="chat-title">Code</h1>
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

      <div className="code-toolbar">
        <div className="workspace-field">
          <IconFolder size={15} />
          <input
            value={workspace}
            onChange={(event) => onWorkspace(event.target.value)}
            placeholder="Đường dẫn workspace, ví dụ /home/ban/du-an"
            spellCheck={false}
            aria-label="Workspace"
          />
          <button className="btn btn-sm" onClick={onOpenWorkspace}>
            Mở
          </button>
        </div>
        <p className="workspace-hint">
          Agent chỉ đọc/ghi file và chạy lệnh bên trong thư mục này.
        </p>
      </div>

      <div className="chat-scroll">
        <div className="thread">
          {error && (
            <div className="error-banner" role="alert">
              <div className="error-copy">
                <strong>Không chạy được yêu cầu</strong>
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
              <Spinner label="Đang mở phiên code…" />
            </div>
          ) : messages.length === 0 ? (
            <section className="welcome code-empty">
              <div className="welcome-mark tone-code" aria-hidden="true">
                <IconTerminal size={22} />
              </div>
              <div className="welcome-eyebrow">WORKSPACE AGENT</div>
              <h2 className="welcome-title">Đọc code, sửa lỗi, chạy test.</h2>
              <p className="welcome-copy">
                tez có thể liệt kê file, đọc nội dung, ghi thay đổi và chạy lệnh build/test — tất cả
                trong workspace bạn chọn.
              </p>

              {!connection && (
                <div className="connect-notice">
                  <span className="connect-icon">
                    <IconKey size={18} />
                  </span>
                  <div className="connect-copy">
                    <strong>Chưa kết nối nhà cung cấp.</strong>
                    <p>Thêm API key của Groq, Gemini hoặc OpenRouter để bắt đầu.</p>
                  </div>
                  <button className="btn btn-primary btn-sm" onClick={onOpenSettings}>
                    Kết nối
                  </button>
                </div>
              )}

              <div className="code-suggestions">
                {CODE_SUGGESTIONS.map((suggestion) => (
                  <button
                    key={suggestion}
                    className="code-suggestion"
                    onClick={() => pickSuggestion(suggestion)}
                  >
                    <IconArrowRight size={15} />
                    <span>{suggestion}</span>
                  </button>
                ))}
              </div>

              <div className="code-note">
                <IconCode size={14} />
                <span>write_file ghi đè toàn bộ file. Agent được dặn đọc trước khi sửa.</span>
              </div>
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

          {busy && messages.length > 0 && (
            <div className="msg msg-assistant is-pending">
              <div className="msg-role">tez</div>
              <div className="msg-body">
                <div className="pending-line">
                  <span className="typing" aria-hidden="true">
                    <span />
                    <span />
                    <span />
                  </span>
                  Đang chạy agent…
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
            placeholder={placeholderFor("general", "code")}
            rows={1}
            aria-label="Yêu cầu cho code agent"
            disabled={loadingSession}
          />
          <div className="composer-foot">
            <div className="composer-hint">
              {busy ? (
                <span>Agent đang chạy…</span>
              ) : (
                <>
                  <kbd>Enter</kbd> gửi <span className="hint-sep">·</span>{" "}
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
                className="send"
                onClick={onSend}
                disabled={!input.trim() || busy || loadingSession}
                aria-label="Gửi yêu cầu"
              >
                <IconArrowUp size={18} />
              </button>
            </div>
          </div>
        </div>
        <p className="composer-note">
          Agent có thể sửa file. Kiểm tra diff và chạy test trước khi commit.
        </p>
      </div>
    </main>
  );
}
