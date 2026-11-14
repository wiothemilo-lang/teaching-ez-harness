import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "./api";
import { errorMessage } from "./api";
import { uid } from "./lib";
import type {
  AppMode,
  AppStatus,
  ApprovalRequest,
  ChatMessage,
  CompactReport,
  SettingsTab,
  SessionInfo,
  ToastItem,
  ToastKind,
  TodoItem,
} from "./types";
import { Sidebar } from "./components/Sidebar";
import { ChatView } from "./components/ChatView";
import { CodeView } from "./components/CodeView";
import { ApprovalDialog } from "./components/ApprovalDialog";
import { SettingsPanel } from "./components/SettingsPanel";
import { Spinner } from "./components/ui";
import { IconCheck, IconInfo, IconRefresh, IconX } from "./icons";

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [mode, setMode] = useState<AppMode>("chat");
  const [subject, setSubject] = useState("general");
  const [grade, setGrade] = useState("10");
  const [workspace, setWorkspace] = useState("");
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
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);
  const [todos, setTodos] = useState<TodoItem[]>([]);
  const [compaction, setCompaction] = useState<CompactReport | null>(null);
  const [denials, setDenials] = useState(0);
  const [contextLimit, setContextLimit] = useState(60_000);

  const notify = useCallback((kind: ToastKind, title: string, detail?: string) => {
    setToasts([{ id: uid(), kind, title, detail }]);
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((previous) => previous.filter((toast) => toast.id !== id));
  }, []);

  const respondApproval = useCallback(
    (approved: boolean) => {
      if (!approval) return;
      const id = approval.id;
      setApproval(null);
      void api.respondApproval(id, approved).catch((responseError) => {
        notify("error", "Không gửi được phản hồi", errorMessage(responseError));
      });
    },
    [approval, notify],
  );

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

  const refreshHarness = useCallback(async () => {
    try {
      const info = await api.getHarness();
      setContextLimit(info.config.compaction.context_char_limit);
    } catch {
      // Bản preview không có backend Tauri; giữ giới hạn mặc định.
    }
  }, []);

  useEffect(() => {
    void refreshHarness();
  }, [refreshHarness]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<ApprovalRequest>("tez://approval-request", (event) => {
      setApproval(event.payload);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {
        // Không có event bridge (preview/web) — bỏ qua.
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!workspace && status?.default_workspace) {
      setWorkspace(status.default_workspace);
    }
  }, [status, workspace]);

  const newSession = useCallback(() => {
    setSessionId(null);
    setMessages([]);
    setInput("");
    setError(null);
    setLastUsage(null);
    setTodos([]);
    setCompaction(null);
    setDenials(0);
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

  const handleChanged = useCallback(async () => {
    await refresh(true);
    await refreshHarness();
  }, [refresh, refreshHarness]);

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

  const handleMode = (next: AppMode) => {
    if (busy || next === mode) return;
    setMode(next);
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
        setTodos(detail.todos ?? []);
        setCompaction(null);
        setDenials(0);
        const found = sessions.find((session) => session.id === id);
        if (found) {
          setMode(found.mode === "code" ? "code" : "chat");
          if (found.subject) setSubject(found.subject);
        }
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
        subject: mode === "code" ? "general" : subject,
        grade,
        message: text,
        session_id: sessionId,
        provider_id: status.connection.id,
        mode,
        workspace: mode === "code" ? workspace || null : null,
      });
      setSessionId(reply.session_id);
      setMessages((previous) => [
        ...previous,
        { id: uid(), role: "assistant", text: reply.text, tools: reply.tools },
      ]);
      setLastUsage({ input: reply.input_tokens, output: reply.output_tokens });
      setTodos(reply.todos ?? []);
      setCompaction(reply.compactions ?? null);
      setDenials(reply.permission_denials ?? 0);
      if (reply.stop_reason === "max_steps") {
        notify(
          "info",
          "Đã chạm giới hạn bước",
          "Agent dừng để tránh chạy quá lâu. Bạn có thể yêu cầu tiếp tục hoặc tăng giới hạn trong Harness.",
        );
      }
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
    mode,
    workspace,
    refresh,
    openSettings,
    notify,
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
        mode={mode}
        subject={subject}
        grade={grade}
        workspace={workspace}
        sessionId={sessionId}
        loadingSession={loadingSession}
        onMode={handleMode}
        onSubject={handleSubject}
        onGrade={setGrade}
        onNewSession={startNewSession}
        onOpenSession={(id) => void openSession(id)}
        onOpenSettings={openSettings}
      />

      {mode === "code" ? (
        <CodeView
          connection={status.connection}
          messages={messages}
          input={input}
          busy={busy}
          error={error}
          loadingSession={loadingSession}
          lastUsage={lastUsage}
          todos={todos}
          compaction={compaction}
          contextLimit={contextLimit}
          denials={denials}
          workspace={workspace}
          onWorkspace={setWorkspace}
          onOpenWorkspace={() => {
            api.openWorkspace(workspace || null).then(
              (info) => setWorkspace(info.path),
              (openError) => notify("error", "Không mở được workspace", errorMessage(openError)),
            );
          }}
          onInput={setInput}
          onSend={() => void send()}
          onOpenSettings={() => openSettings("providers")}
          onOpenHarness={() => openSettings("harness")}
          onDismissError={() => setError(null)}
        />
      ) : (
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
          todos={todos}
          compaction={compaction}
          contextLimit={contextLimit}
          denials={denials}
          onInput={setInput}
          onSend={() => void send()}
          onOpenSettings={() => openSettings("providers")}
          onOpenHarness={() => openSettings("harness")}
          onDismissError={() => setError(null)}
        />
      )}

      <SettingsPanel
        open={settingsOpen}
        tab={settingsTab}
        status={status}
        onTab={setSettingsTab}
        onClose={closeSettings}
        onChanged={handleChanged}
        notify={notify}
      />

      {approval && <ApprovalDialog request={approval} onRespond={respondApproval} />}

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
