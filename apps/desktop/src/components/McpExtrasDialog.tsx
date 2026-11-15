import { useEffect, useState } from "react";
import * as api from "../api";
import type { ToastKind } from "../types";
import { IconX } from "../icons";

type Props = {
  open: boolean;
  serverName: string | null;
  onClose: () => void;
  notify: (kind: ToastKind, title: string, detail?: string) => void;
};

const ACTIONS = [
  { id: "list_resources", label: "Liệt kê tài nguyên" },
  { id: "read_resource", label: "Đọc tài nguyên" },
  { id: "list_prompts", label: "Liệt kê prompt" },
  { id: "get_prompt", label: "Lấy prompt" },
] as const;

/** Xem resources/prompts do MCP server cung cấp mà không cần rời Settings. */
export function McpExtrasDialog({ open, serverName, onClose, notify }: Props) {
  const [action, setAction] = useState<string>("list_resources");
  const [uri, setUri] = useState("");
  const [promptName, setPromptName] = useState("");
  const [argsText, setArgsText] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");

  useEffect(() => {
    if (!open) return;
    setAction("list_resources");
    setUri("");
    setPromptName("");
    setArgsText("{}");
    setBusy(false);
    setResult("");
  }, [open, serverName]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open || !serverName) return null;

  const run = async () => {
    let args: Record<string, unknown> | undefined;
    if (action === "get_prompt") {
      try {
        const parsed: unknown = JSON.parse(argsText || "{}");
        if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
          throw new Error("arguments phải là object");
        }
        args = parsed as Record<string, unknown>;
      } catch (error) {
        notify("error", "JSON không hợp lệ", api.errorMessage(error));
        return;
      }
    }

    setBusy(true);
    setResult("");
    try {
      const text = await api.mcpExtras(
        serverName,
        action,
        action === "read_resource" ? uri.trim() : undefined,
        action === "get_prompt" ? promptName.trim() : undefined,
        args,
      );
      setResult(text);
    } catch (error) {
      notify("error", "Không đọc được tài nguyên MCP", api.errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="approval-backdrop" role="dialog" aria-modal="true" aria-labelledby="mcp-extras-title">
      <div className="approval-card">
        <div className="approval-head">
          <div className="approval-copy">
            <p className="approval-kicker">MCP · {serverName}</p>
            <h2 id="mcp-extras-title">Tài nguyên &amp; prompt</h2>
          </div>
          <button type="button" className="approval-close" aria-label="Đóng" onClick={onClose}>
            <IconX size={16} />
          </button>
        </div>

        <div className="field-grid">
          <label className="field">
            <span>Hành động</span>
            <select value={action} onChange={(event) => setAction(event.target.value)}>
              {ACTIONS.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>
          {action === "read_resource" && (
            <label className="field">
              <span>URI</span>
              <input
                value={uri}
                onChange={(event) => setUri(event.target.value)}
                placeholder="file:///..."
              />
            </label>
          )}
          {action === "get_prompt" && (
            <>
              <label className="field">
                <span>Tên prompt</span>
                <input
                  value={promptName}
                  onChange={(event) => setPromptName(event.target.value)}
                  placeholder="summarize"
                />
              </label>
              <label className="field field-wide">
                <span>Arguments (JSON)</span>
                <textarea
                  rows={3}
                  value={argsText}
                  onChange={(event) => setArgsText(event.target.value)}
                  spellCheck={false}
                />
              </label>
            </>
          )}
        </div>

        <div className="row-actions">
          <button
            className="btn btn-sm btn-primary"
            onClick={() => void run()}
            disabled={busy || (action === "read_resource" && !uri.trim()) || (action === "get_prompt" && !promptName.trim())}
          >
            {busy ? "Đang đọc…" : "Chạy"}
          </button>
          <button className="btn btn-sm btn-ghost" onClick={onClose}>
            Đóng
          </button>
        </div>

        {result && <pre className="approval-args">{result}</pre>}
      </div>
    </div>
  );
}
