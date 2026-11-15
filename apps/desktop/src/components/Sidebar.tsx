import type { AppMode, AppStatus, SessionInfo, SettingsTab } from "../types";
import { MODE_LABELS, relativeTime } from "../lib";
import {
  IconChat,
  IconChevronRight,
  IconCode,
  IconFolder,
  IconPlan,
  IconPlus,
  IconSearch,
  IconSliders,
} from "../icons";
import { Spinner } from "./ui";

type Props = {
  status: AppStatus;
  sessions: SessionInfo[];
  mode: AppMode;
  workspace: string;
  sessionId: string | null;
  loadingSession: boolean;
  onMode: (mode: AppMode) => void;
  onNewSession: () => void;
  onOpenSession: (id: string) => void;
  onOpenSettings: (tab: SettingsTab) => void;
  onCommand: () => void;
  onOpenWorkspace: () => void;
};

export function Sidebar({
  status,
  sessions,
  mode,
  workspace,
  sessionId,
  loadingSession,
  onMode,
  onNewSession,
  onOpenSession,
  onOpenSettings,
  onCommand,
  onOpenWorkspace,
}: Props) {
  const modes: AppMode[] = ["chat", "plan", "code"];

  return (
    <aside className="sidebar">
      <header className="brand">
        <div className="brand-mark" aria-hidden="true">
          t
        </div>
        <div className="brand-copy">
          <div className="brand-name">tez</div>
          <div className="brand-sub">agent workspace</div>
        </div>
      </header>

      <button className="command-trigger" onClick={onCommand}>
        <IconSearch size={15} />
        <span>Tìm hoặc chạy lệnh…</span>
        <kbd>Ctrl K</kbd>
      </button>

      <button className="new-session" onClick={onNewSession}>
        <IconPlus size={15} />
        <span>Cuộc trò chuyện mới</span>
        <kbd>Ctrl N</kbd>
      </button>

      <div className="mode-switch" role="tablist" aria-label="Chế độ làm việc">
        {modes.map((item) => (
          <button
            key={item}
            role="tab"
            aria-selected={mode === item}
            className={`mode-option ${mode === item ? "is-active" : ""}`}
            onClick={() => onMode(item)}
          >
            {item === "chat" ? (
              <IconChat size={15} />
            ) : item === "plan" ? (
              <IconPlan size={15} />
            ) : (
              <IconCode size={15} />
            )}
            <span>{MODE_LABELS[item]}</span>
          </button>
        ))}
      </div>

      <div className="side-section workspace-section">
        <div className="side-label">Workspace</div>
        <button className="workspace-card" onClick={onOpenWorkspace} title={workspace || "Chọn workspace"}>
          <IconFolder size={15} />
          <span className="workspace-copy">
            <strong>{workspace ? shortPath(workspace) : "Chưa chọn thư mục"}</strong>
            <small>
              {mode === "code"
                ? "Agent đọc/ghi file trong thư mục này"
                : mode === "plan"
                  ? "Chỉ đọc để khảo sát và lập kế hoạch"
                  : "Dùng cho chỉ dẫn dự án và ngữ cảnh"}
            </small>
          </span>
          <IconChevronRight size={14} />
        </button>
      </div>

      <div className="side-section sessions-section">
        <div className="side-label sessions-label">
          <span>Gần đây</span>
          {loadingSession && <Spinner />}
        </div>
        <div className="session-list">
          {sessions.length === 0 ? (
            <p className="side-empty">Chưa có phiên nào.</p>
          ) : (
            sessions.map((session) => {
              const active = session.id === sessionId;
              return (
                <button
                  key={session.id}
                  className={`session ${active ? "is-active" : ""}`}
                  onClick={() => onOpenSession(session.id)}
                  title={session.summary ?? undefined}
                >
                  <span className="session-head">
                    <span className={`session-mode mode-${session.mode}`}>
                      {session.mode === "code" ? "CODE" : session.mode === "plan" ? "PLAN" : "CHAT"}
                    </span>
                    <span className="session-title">
                      {session.summary?.trim() ||
                        (session.mode === "code"
                          ? "Phiên code"
                          : session.mode === "plan"
                            ? "Phiên plan"
                            : "Hội thoại")}
                    </span>
                  </span>
                  <span className="session-meta">
                    {session.messages} tin · {relativeTime(session.started_at)}
                  </span>
                </button>
              );
            })
          )}
        </div>
      </div>

      <footer className="side-footer">
        <button className="provider-status" onClick={() => onOpenSettings("providers")}>
          <span className={`status-dot ${status.connection ? "is-on" : ""}`} aria-hidden="true" />
          <span className="provider-status-copy">
            <span className="provider-status-name">
              {status.connection ? status.connection.name : "Chưa kết nối"}
            </span>
            <span className="provider-status-meta">
              {status.connection ? status.connection.model : "Thêm API key để bắt đầu"}
            </span>
          </span>
          <IconChevronRight size={14} />
        </button>
        <button className="side-settings" onClick={() => onOpenSettings("harness")}>
          <IconSliders size={16} />
          <span>Cài đặt agent</span>
          <span className="side-counts">v{status.version}</span>
        </button>
      </footer>
    </aside>
  );
}

function shortPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return path;
  return `…/${parts.slice(-2).join("/")}`;
}
