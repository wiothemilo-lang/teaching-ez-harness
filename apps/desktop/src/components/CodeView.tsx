import { useEffect, useRef, useState } from "react";
import type { ChangeEvent, KeyboardEvent } from "react";
import type { ChatMessage, CompactReport, ConnectionInfo, TodoItem } from "../types";
import { CODE_SUGGESTIONS, placeholderFor } from "../lib";
import {
  IconArrowRight,
  IconArrowUp,
  IconCode,
  IconFolder,
  IconImage,
  IconKey,
  IconSliders,
  IconStop,
  IconTerminal,
  IconX,
} from "../icons";
import { ToolTraces } from "./ChatView";
import { HarnessDock } from "./HarnessDock";
import { Spinner } from "./ui";

type Props = {
  connection: ConnectionInfo | null;
  messages: ChatMessage[];
  images: string[];
  onImagesChange: (images: string[]) => void;
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
  workspace: string;
  onWorkspace: (value: string) => void;
  onOpenWorkspace: () => void;
  onInput: (value: string) => void;
  onSend: () => void;
  onStop: () => void;
  onOpenSettings: () => void;
  onOpenHarness: () => void;
  onDismissError: () => void;
};

const MAX_IMAGES = 4;
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/webp", "image/gif"]);

function readImage(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error ?? new Error("không đọc được ảnh"));
    reader.readAsDataURL(file);
  });
}

export function CodeView({
  connection,
  messages,
  images,
  onImagesChange,
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
  workspace,
  onWorkspace,
  onOpenWorkspace,
  onInput,
  onSend,
  onStop,
  onOpenSettings,
  onOpenHarness,
  onDismissError,
}: Props) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);

  useEffect(() => {
    const element = textareaRef.current;
    if (!element) return;
    element.style.height = "0px";
    element.style.height = `${Math.min(element.scrollHeight, 220)}px`;
  }, [input]);

  const addFiles = async (event: ChangeEvent<HTMLInputElement>) => {
    const files = event.target.files;
    if (!files || files.length === 0) return;
    const room = MAX_IMAGES - images.length;
    if (room <= 0) {
      setAttachmentError(`Tối đa ${MAX_IMAGES} ảnh mỗi lượt.`);
      event.target.value = "";
      return;
    }
    const picked = Array.from(files).slice(0, room);
    const valid = picked.filter((file) => IMAGE_TYPES.has(file.type));
    const oversized = valid.find((file) => file.size > MAX_IMAGE_BYTES);
    if (oversized) {
      setAttachmentError(`Ảnh "${oversized.name}" vượt quá 5 MB.`);
      event.target.value = "";
      return;
    }
    if (valid.length === 0) {
      setAttachmentError("Chỉ hỗ trợ PNG, JPEG, WebP hoặc GIF.");
      event.target.value = "";
      return;
    }
    try {
      const urls = await Promise.all(valid.map(readImage));
      onImagesChange([...images, ...urls]);
      setAttachmentError(null);
    } catch (readError) {
      setAttachmentError(readError instanceof Error ? readError.message : "Không đọc được ảnh.");
    } finally {
      event.target.value = "";
    }
  };

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
                  {message.images && message.images.length > 0 && (
                    <div className="msg-images">
                      {message.images.map((src, index) => (
                        <img key={index} src={src} alt={`Ảnh ${index + 1}`} />
                      ))}
                    </div>
                  )}
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
          {images.length > 0 && (
            <div className="attachments">
              {images.map((src, index) => (
                <div className="attachment" key={index}>
                  <img src={src} alt={`Ảnh ${index + 1}`} />
                  <button
                    type="button"
                    className="attachment-remove"
                    onClick={() => onImagesChange(images.filter((_, at) => at !== index))}
                    aria-label={`Bỏ ảnh ${index + 1}`}
                  >
                    <IconX size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}
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
          <input
            ref={fileRef}
            type="file"
            accept="image/*"
            multiple
            hidden
            onChange={(event) => void addFiles(event)}
          />
          <div className="composer-foot">
            <div className="composer-hint">
              {!busy && (
                <button
                  type="button"
                  className="attach-button"
                  onClick={() => fileRef.current?.click()}
                  disabled={loadingSession || images.length >= MAX_IMAGES}
                  title={`Đính kèm ảnh (tối đa ${MAX_IMAGES})`}
                >
                  <IconImage size={14} />
                  Ảnh
                </button>
              )}
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
                className={`send ${busy ? "is-stop" : ""}`}
                onClick={busy ? onStop : onSend}
                disabled={!busy && (!input.trim() || loadingSession)}
                aria-label={busy ? "Dừng agent" : "Gửi yêu cầu"}
              >
                {busy ? <IconStop size={16} /> : <IconArrowUp size={18} />}
              </button>
            </div>
          </div>
        </div>
        {attachmentError && <p className="composer-error">{attachmentError}</p>}
        <p className="composer-note">
          Agent có thể sửa file. Kiểm tra diff và chạy test trước khi commit.
        </p>
      </div>
    </main>
  );
}
