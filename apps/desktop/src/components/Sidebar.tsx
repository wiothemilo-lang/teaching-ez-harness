import type { AppStatus, SessionInfo, SettingsTab } from "../types";
import { relativeTime, subjectName } from "../lib";
import { IconChevronRight, IconPlus, IconSliders, subjectIcon } from "../icons";
import { Spinner } from "./ui";

type Props = {
  status: AppStatus;
  sessions: SessionInfo[];
  subject: string;
  grade: string;
  sessionId: string | null;
  loadingSession: boolean;
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
  subject,
  grade,
  sessionId,
  loadingSession,
  onSubject,
  onGrade,
  onNewSession,
  onOpenSession,
  onOpenSettings,
}: Props) {
  return (
    <aside className="sidebar">
      <header className="brand">
        <div className="brand-mark" aria-hidden="true">
          t
        </div>
        <div className="brand-copy">
          <div className="brand-name">tez</div>
          <div className="brand-sub">gia sư local-first</div>
        </div>
      </header>

      <button className="new-session" onClick={onNewSession}>
        <IconPlus size={15} />
        <span>Buổi học mới</span>
        <kbd>Ctrl N</kbd>
      </button>

      <div className="side-section">
        <div className="side-label">Môn học</div>
        <nav className="subject-list" aria-label="Môn học">
          {status.subjects.map((item) => {
            const active = item.id === subject;
            return (
              <button
                key={item.id}
                className={`subject ${active ? "is-active" : ""}`}
                onClick={() => onSubject(item.id)}
                aria-current={active ? "true" : undefined}
              >
                <span className="subject-icon">{subjectIcon(item.id, 17)}</span>
                <span className="subject-name">{item.name}</span>
                {active && <span className="subject-dot" aria-hidden="true" />}
              </button>
            );
          })}
        </nav>
      </div>

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

      <div className="side-section sessions-section">
        <div className="side-label sessions-label">
          <span>Gần đây</span>
          {loadingSession && <Spinner />}
        </div>
        <div className="session-list">
          {sessions.length === 0 ? (
            <p className="side-empty">Chưa có buổi học nào.</p>
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
                  <span className="session-title">
                    {subjectName(status.subjects, session.subject)}
                  </span>
                  <span className="session-meta">
                    {session.messages} tin nhắn · {relativeTime(session.started_at)}
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
          <span className="side-counts">
            {status.extensions.skills} kỹ năng · {status.extensions.mcp_servers} MCP
          </span>
        </button>
      </footer>
    </aside>
  );
}
