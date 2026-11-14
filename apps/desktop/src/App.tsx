import { useCallback, useEffect, useState } from "react";
import * as api from "./api";
import { errorMessage } from "./api";
import { uid } from "./lib";
import type {
  AppStatus,
  ChatMessage,
  SettingsTab,
  SessionInfo,
  ToastItem,
  ToastKind,
} from "./types";
import { Sidebar } from "./components/Sidebar";
import { ChatView } from "./components/ChatView";
import { SettingsPanel } from "./components/SettingsPanel";
import { Spinner } from "./components/ui";
import { IconCheck, IconInfo, IconRefresh, IconX } from "./icons";

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [subject, setSubject] = useState("math");
  const [grade, setGrade] = useState("10");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastUsage, setLastUsage] = useState<{ input: number; output: number } | null>(null);
  const [loadingSession, setLoadingSession] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("providers");
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const notify = useCallback((kind: ToastKind, title: string, detail?: string) => {
    setToasts([{ id: uid(), kind, title, detail }]);
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((previous) => previous.filter((toast) => toast.id !== id));
  }, []);

  const refresh = useCallback(
    async (silent = false) => {
      try {
        const [nextStatus, nextSessions] = await Promise.all([
          api.appStatus(),
          api.listSessions(20),
        ]);
        setStatus(nextStatus);
        setSessions(nextSessions);
        setStatusError(null);
      } catch (refreshError) {
        const message = errorMessage(refreshError);
        setStatusError(message);
        if (!silent) notify("error", "Không tải được trạng thái", message);
      }
    },
    [notify],
  );

  useEffect(() => {
    void refresh(true);
  }, [refresh]);

  const newSession = useCallback(() => {
    setSessionId(null);
    setMessages([]);
    setInput("");
    setError(null);
    setLastUsage(null);
  }, []);

  const startNewSession = useCallback(() => {
    if (busy) return;
    newSession();
  }, [busy, newSession]);

  const openSettings = useCallback((tab: SettingsTab = "providers") => {
    setSettingsTab(tab);
    setSettingsOpen(true);
  }, []);

  const closeSettings = useCallback(() => setSettingsOpen(false), []);

  const handleChanged = useCallback(() => {
    void refresh(true);
  }, [refresh]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const modifier = event.ctrlKey || event.metaKey;
      if (modifier && event.key === ",") {
        event.preventDefault();
        openSettings("providers");
      }
      if (modifier && event.key.toLowerCase() === "n") {
        event.preventDefault();
        startNewSession();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openSettings, startNewSession]);

  const handleSubject = (id: string) => {
    if (busy || id === subject) return;
    setSubject(id);
    newSession();
  };

  const openSession = useCallback(
    async (id: string) => {
      if (busy) return;
      setLoadingSession(true);
      setError(null);
      try {
        const detail = await api.loadSession(id);
        setSessionId(detail.id);
        setMessages(
          detail.messages
            .filter((message) => message.role === "user" || message.role === "assistant")
            .map((message) => ({
              id: uid(),
              role: message.role === "user" ? ("user" as const) : ("assistant" as const),
              text: message.content,
            })),
        );
        setLastUsage(null);
        const found = sessions.find((session) => session.id === id);
        if (found?.subject) setSubject(found.subject);
      } catch (openError) {
        notify("error", "Không mở được buổi học", errorMessage(openError));
      } finally {
        setLoadingSession(false);
      }
    },
    [busy, sessions, notify],
  );

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || busy || loadingSession) return;

    if (!status?.connection) {
      setError("Chưa kết nối nhà cung cấp. Thêm API key trong Cài đặt → Nhà cung cấp.");
      openSettings("providers");
      return;
    }

    const optimisticId = uid();
    setMessages((previous) => [
      ...previous,
      { id: optimisticId, role: "user", text },
    ]);
    setInput("");
    setBusy(true);
    setError(null);

    try {
      const reply = await api.chat({
        subject,
        grade,
        message: text,
        session_id: sessionId,
        provider_id: status.connection.id,
      });
      setSessionId(reply.session_id);
      setMessages((previous) => [
        ...previous,
        { id: uid(), role: "assistant", text: reply.text, tools: reply.tools },
      ]);
      setLastUsage({ input: reply.input_tokens, output: reply.output_tokens });
      void refresh(true);
    } catch (sendError) {
      setMessages((previous) => previous.filter((message) => message.id !== optimisticId));
      setInput(text);
      setError(errorMessage(sendError));
    } finally {
      setBusy(false);
    }
  }, [
    input,
    busy,
    loadingSession,
    status,
    subject,
    grade,
    sessionId,
    refresh,
    openSettings,
  ]);

  if (!status) {
    return (
      <div className="boot">
        <div className="boot-card">
          <div className="brand-mark boot-mark" aria-hidden="true">
            t
          </div>
          {statusError ? (
            <>
              <h1>Không mở được bàn học</h1>
              <p>{statusError}</p>
              <button className="btn" onClick={() => void refresh()}>
                <IconRefresh size={15} />
                Thử lại
              </button>
            </>
          ) : (
            <>
              <Spinner label="Đang mở bàn học…" />
              <p className="boot-note">Đọc dữ liệu local và nhà cung cấp đã lưu.</p>
            </>
          )}
        </div>
      </div>
    );
  }

  const subjectInfo = status.subjects.find((item) => item.id === subject);

  return (
    <div className="app">
      <Sidebar
        status={status}
        sessions={sessions}
        subject={subject}
        grade={grade}
        sessionId={sessionId}
        loadingSession={loadingSession}
        onSubject={handleSubject}
        onGrade={setGrade}
        onNewSession={startNewSession}
        onOpenSession={(id) => void openSession(id)}
        onOpenSettings={openSettings}
      />

      <ChatView
        subject={subjectInfo}
        grade={grade}
        connection={status.connection}
        messages={messages}
        input={input}
        busy={busy}
        error={error}
        loadingSession={loadingSession}
        lastUsage={lastUsage}
        onInput={setInput}
        onSend={() => void send()}
        onOpenSettings={() => openSettings("providers")}
        onDismissError={() => setError(null)}
      />

      <SettingsPanel
        open={settingsOpen}
        tab={settingsTab}
        status={status}
        onTab={setSettingsTab}
        onClose={closeSettings}
        onChanged={handleChanged}
        notify={notify}
      />

      <div className="toasts" aria-live="polite" aria-atomic="false">
        {toasts.map((toast) => (
          <ToastCard key={toast.id} toast={toast} onDismiss={dismissToast} />
        ))}
      </div>
    </div>
  );
}

function ToastCard({ toast, onDismiss }: { toast: ToastItem; onDismiss: (id: string) => void }) {
  useEffect(() => {
    const timer = window.setTimeout(() => onDismiss(toast.id), 5000);
    return () => window.clearTimeout(timer);
  }, [toast.id, onDismiss]);

  return (
    <div className={`toast toast-${toast.kind}`} role="status">
      <span className="toast-icon" aria-hidden="true">
        {toast.kind === "success" ? (
          <IconCheck size={15} />
        ) : toast.kind === "error" ? (
          <IconX size={15} />
        ) : (
          <IconInfo size={15} />
        )}
      </span>
      <div className="toast-copy">
        <strong>{toast.title}</strong>
        {toast.detail && <p>{toast.detail}</p>}
      </div>
      <button
        className="icon-button toast-close"
        onClick={() => onDismiss(toast.id)}
        aria-label="Đóng thông báo"
      >
        <IconX size={13} />
      </button>
    </div>
  );
}
