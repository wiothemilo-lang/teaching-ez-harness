import type { AppMode, AppStatus, SessionInfo, SettingsTab } from "../types";
import { MODE_LABELS, relativeTime, subjectName } from "../lib";
import {
  IconChat,
  IconChevronRight,
  IconCode,
  IconPlus,
  IconSliders,
  subjectIcon,
} from "../icons";
import { Spinner } from "./ui";

type Props = {
  status: AppStatus;
  sessions: SessionInfo[];
  mode: AppMode;
  subject: string;
  grade: string;
  workspace: string;
  sessionId: string | null;
  loadingSession: boolean;
  onMode: (mode: AppMode) => void;
  onSubject: (id: string) => void;
  onGrade: (grade: string) => void;
  onNewSession: () => void;
  onOpenSession: (id: string) => void;
  onOpenSettings: (tab: SettingsTab) => void;
};

const GRADES = ["10", "11", "12"];

export function Sidebar({
  status,
  sessions,
  mode,
  subject,
  grade,
  workspace,
  sessionId,
  loadingSession,
  onMode,
  onSubject,
  onGrade,
  onNewSession,
  onOpenSession,
  onOpenSettings,
}: Props) {
  const modes: AppMode[] = ["chat", "code"];

  return (
    <aside className="sidebar">
      <header className="brand">
        <div className="brand-mark" aria-hidden="true">
          t
        </div>
        <div className="brand-copy">
          <div className="brand-name">tez</div>
          <div className="brand-sub">local-first workspace</div>
        </div>
      </header>

      <div className="mode-switch" role="tablist" aria-label="Chế độ làm việc">
        {modes.map((item) => (
          <button
            key={item}
            role="tab"
            aria-selected={mode === item}
            className={`mode-option ${mode === item ? "is-active" : ""}`}
            onClick={() => onMode(item)}
          >
            {item === "chat" ? <IconChat size={15} /> : <IconCode size={15} />}
            <span>{MODE_LABELS[item]}</span>
          </button>
        ))}
      </div>

      <button className="new-session" onClick={onNewSession}>
        <IconPlus size={15} />
        <span>{mode === "code" ? "Phiên code mới" : "Hội thoại mới"}</span>
        <kbd>Ctrl N</kbd>
      </button>

      {mode === "chat" ? (
        <>
          <div className="side-section">
            <div className="side-label">Chủ đề</div>
            <nav className="subject-list" aria-label="Chủ đề">
              {status.subjects.map((item) => {
                const active = item.id === subject;
                return (
                  <button
                    key={item.id}
                    className={`subject ${active ? "is-active" : ""}`}
                    onClick={() => onSubject(item.id)}
                    aria-current={active ? "true" : undefined}
                  >
                    <span className="subject-icon">{subjectIcon(item.id, 16)}</span>
                    <span className="subject-name">{item.name}</span>
                    {active && <span className="subject-dot" aria-hidden="true" />}
                  </button>
                );
              })}
            </nav>
          </div>

          {subject !== "general" && (
            <div className="side-section">
              <div className="side-label">Lớp</div>
              <div className="grade-group" role="group" aria-label="Lớp">
                {GRADES.map((item) => (
                  <button
                    key={item}
                    className={item === grade ? "is-active" : ""}
                    onClick={() => onGrade(item)}
                    aria-pressed={item === grade}
                  >
                    {item}
                  </button>
                ))}
              </div>
            </div>
          )}
        </>
      ) : (
        <div className="side-section">
          <div className="side-label">Workspace</div>
          <div className="workspace-chip" title={workspace || "Workspace mặc định"}>
            <IconCode size={14} />
            <code>{workspace ? shortPath(workspace) : "mặc định"}</code>
          </div>
          <p className="side-hint">Agent chỉ đọc/ghi trong thư mục này.</p>
        </div>
      )}

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
                  title={subjectName(status.subjects, session.subject)}
                >
                  <span className="session-head">
                    <span className={`session-mode mode-${session.mode}`}>
                      {session.mode === "code" ? "CODE" : "CHAT"}
                    </span>
                    <span className="session-title">
                      {session.mode === "code"
                        ? "Phiên code"
                        : subjectName(status.subjects, session.subject)}
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
        <button className="side-settings" onClick={() => onOpenSettings("providers")}>
          <IconSliders size={16} />
          <span>Cài đặt</span>
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
