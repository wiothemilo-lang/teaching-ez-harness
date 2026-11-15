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
  ExtensionKind,
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
import { CommandPalette } from "./components/CommandPalette";
import { PlanDialog } from "./components/PlanDialog";
import { SettingsPanel } from "./components/SettingsPanel";
import { Spinner } from "./components/ui";
import { IconCheck, IconInfo, IconRefresh, IconX } from "./icons";

const STREAM_MESSAGE_ID = "streaming";

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [mode, setMode] = useState<AppMode>("chat");
  const [workspace, setWorkspace] = useState("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [images, setImages] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastUsage, setLastUsage] = useState<{ input: number; output: number } | null>(null);
  const [loadingSession, setLoadingSession] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("providers");
  const [commandOpen, setCommandOpen] = useState(false);
  const [planOpen, setPlanOpen] = useState(false);
  const [createRequest, setCreateRequest] = useState<ExtensionKind | null>(null);
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

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<{ session_id: string; text: string }>("tez://chat-delta", (event) => {
      if (disposed) return;
      const text = event.payload.text;
      if (!text) return;
      setMessages((previous) => {
        const index = previous.findIndex((message) => message.id === STREAM_MESSAGE_ID);
        if (index === -1) {
          return [...previous, { id: STREAM_MESSAGE_ID, role: "assistant", text }];
        }
        const next = previous.slice();
        next[index] = { ...next[index], text: next[index].text + text };
        return next;
      });
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const respondApproval = useCallback(
    (approved: boolean, remember: boolean) => {
      if (!approval) return;
      const id = approval.id;
      setApproval(null);
      void api.respondApproval(id, approved, remember).catch((responseError) => {
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
    setImages([]);
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
      if (modifier && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openSettings, startNewSession]);

  const handleMode = (next: AppMode) => {
    if (busy || next === mode) return;
    // Plan ↔ Code giữ nguyên phiên để kế hoạch, todos và hội thoại đi theo.
    const keepSession =
      (mode === "plan" && next === "code") || (mode === "code" && next === "plan");
    setMode(next);
    if (!keepSession) newSession();
  };

  const openWorkspace = useCallback(() => {
    api.openWorkspace(workspace || null).then(
      (info) => setWorkspace(info.path),
      (openError) => notify("error", "Không mở được workspace", errorMessage(openError)),
    );
  }, [workspace, notify]);

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
              images: message.images?.length ? message.images : undefined,
            })),
        );
        setLastUsage(null);
        setImages([]);
        setTodos(detail.todos ?? []);
        setCompaction(null);
        setDenials(0);
        const found = sessions.find((session) => session.id === id);
        if (found) {
          setMode(found.mode === "code" ? "code" : found.mode === "plan" ? "plan" : "chat");
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
    const attached = images;
    setMessages((previous) => [
      ...previous,
      { id: optimisticId, role: "user", text, images: attached },
    ]);
    setInput("");
    setImages([]);
    setBusy(true);
    setError(null);

    try {
      const reply = await api.chat({
        message: text,
        images: attached,
        session_id: sessionId,
        provider_id: status.connection.id,
        mode,
        workspace: workspace || null,
      });
      setSessionId(reply.session_id);
      setMessages((previous) => [
        ...previous.filter((message) => message.id !== STREAM_MESSAGE_ID),
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
          "Agent dừng để tránh chạy quá lâu. Bạn có thể yêu cầu tiếp tục hoặc tăng giới hạn trong Agent.",
        );
      } else if (reply.stop_reason === "stopped") {
        notify("info", "Đã dừng", "Agent dừng theo yêu cầu của bạn.");
      }
      void refresh(true);
    } catch (sendError) {
      setMessages((previous) =>
        previous.filter(
          (message) => message.id !== optimisticId && message.id !== STREAM_MESSAGE_ID,
        ),
      );
      setInput(text);
      setImages(attached);
      setError(errorMessage(sendError));
    } finally {
      setBusy(false);
    }
  }, [
    input,
    busy,
    loadingSession,
    status,
    sessionId,
    images,
    mode,
    workspace,
    refresh,
    openSettings,
    notify,
  ]);

  const stop = useCallback(() => {
    void api.stopChat().catch((stopError) => {
      notify("error", "Không dừng được agent", errorMessage(stopError));
    });
  }, [notify]);

  if (!status) {
    return (
      <div className="boot">
        <div className="boot-card">
          <div className="brand-mark boot-mark" aria-hidden="true">
            t
          </div>
          {statusError ? (
            <>
              <h1>Không mở được agent</h1>
              <p>{statusError}</p>
              <button className="btn" onClick={() => void refresh()}>
                <IconRefresh size={15} />
                Thử lại
              </button>
            </>
          ) : (
            <>
              <Spinner label="Đang mở agent…" />
              <p className="boot-note">Đọc dữ liệu local và nhà cung cấp đã lưu.</p>
            </>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="app">
      <Sidebar
        status={status}
        sessions={sessions}
        mode={mode}
        workspace={workspace}
        sessionId={sessionId}
        loadingSession={loadingSession}
        onMode={handleMode}
        onNewSession={startNewSession}
        onOpenSession={(id) => void openSession(id)}
        onOpenSettings={openSettings}
        onCommand={() => setCommandOpen(true)}
        onOpenWorkspace={openWorkspace}
      />

      {mode === "code" ? (
        <CodeView
          connection={status.connection}
          messages={messages}
          images={images}
          onImagesChange={setImages}
          input={input}
          busy={busy}
          streaming={messages.some((message) => message.id === STREAM_MESSAGE_ID)}
          error={error}
          loadingSession={loadingSession}
          lastUsage={lastUsage}
          todos={todos}
          compaction={compaction}
          contextLimit={contextLimit}
          denials={denials}
          workspace={workspace}
          onWorkspace={setWorkspace}
          onOpenWorkspace={openWorkspace}
          onInput={setInput}
          onSend={() => void send()}
          onStop={stop}
          onOpenSettings={() => openSettings("providers")}
          onOpenHarness={() => openSettings("harness")}
          onDismissError={() => setError(null)}
        />
      ) : (
        <ChatView
          connection={status.connection}
          planMode={mode === "plan"}
          messages={messages}
          images={images}
          onImagesChange={setImages}
          input={input}
          busy={busy}
          streaming={messages.some((message) => message.id === STREAM_MESSAGE_ID)}
          error={error}
          loadingSession={loadingSession}
          lastUsage={lastUsage}
          todos={todos}
          compaction={compaction}
          contextLimit={contextLimit}
          denials={denials}
          onInput={setInput}
          onSend={() => void send()}
          onStop={stop}
          onOpenPlan={() => setPlanOpen(true)}
          onOpenSettings={() => openSettings("providers")}
          onOpenHarness={() => openSettings("harness")}
          onDismissError={() => setError(null)}
        />
      )}

      <CommandPalette
        open={commandOpen}
        onClose={() => setCommandOpen(false)}
        onNewSession={startNewSession}
        onMode={handleMode}
        onOpenSettings={openSettings}
        onOpenWorkspace={openWorkspace}
        onCreate={(kind) => {
          setCommandOpen(false);
          setCreateRequest(kind);
          openSettings(kind === "skill" ? "skills" : kind === "mcp" ? "mcp" : "plugins");
        }}
      />

      <SettingsPanel
        open={settingsOpen}
        tab={settingsTab}
        status={status}
        onTab={setSettingsTab}
        onClose={closeSettings}
        onChanged={handleChanged}
        notify={notify}
        createRequest={createRequest}
        onCreateRequest={() => setCreateRequest(null)}
      />

      <PlanDialog
        open={planOpen}
        sessionId={sessionId}
        onClose={() => setPlanOpen(false)}
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
