import { useEffect, useMemo, useState } from "react";
import * as api from "../api";
import type { DraftExtensionResult, ExtensionKind, ToastKind } from "../types";
import { IconSparkle, IconX } from "../icons";
import { Spinner } from "./ui";

type Props = {
  open: boolean;
  kind: ExtensionKind | null;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
  notify: (kind: ToastKind, title: string, detail?: string) => void;
};

const KIND_META: Record<ExtensionKind, { label: string; placeholder: string; hint: string }> = {
  skill: {
    label: "Skill",
    placeholder:
      "Ví dụ: một skill giúp agent review code Rust, luôn kiểm tra lỗi ownership trước khi đề xuất sửa…",
    hint: "AI sẽ viết SKILL.md với frontmatter name, description, subjects, triggers.",
  },
  mcp: {
    label: "MCP server",
    placeholder:
      "Ví dụ: server filesystem chỉ cho phép đọc trong /root/teaching-ez-harness, dùng npx…",
    hint: "AI sẽ đề xuất command, args và env cho một MCP server stdio.",
  },
  plugin: {
    label: "Plugin",
    placeholder:
      "Ví dụ: plugin đóng gói skill viết changelog và một MCP server đọc git log…",
    hint: "AI sẽ tạo plugin.json và skill đi kèm trong plugins/<id>/.",
  },
};

export function CreateWithAI({ open, kind, onClose, onSaved, notify }: Props) {
  const [prompt, setPrompt] = useState("");
  const [draft, setDraft] = useState<DraftExtensionResult | null>(null);
  const [activeFile, setActiveFile] = useState(0);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setPrompt("");
    setDraft(null);
    setActiveFile(0);
    setError(null);
  }, [open, kind]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && !loading && !saving) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, loading, saving, onClose]);

  const meta = useMemo(() => (kind ? KIND_META[kind] : null), [kind]);

  if (!open || !kind || !meta) return null;

  const generate = async () => {
    if (!prompt.trim() || loading) return;
    setLoading(true);
    setError(null);
    setDraft(null);
    try {
      const result = await api.draftExtension(kind, prompt.trim());
      setDraft(result);
      setActiveFile(0);
    } catch (generateError) {
      setError(api.errorMessage(generateError));
    } finally {
      setLoading(false);
    }
  };

  const save = async () => {
    if (!draft || saving) return;
    setSaving(true);
    setError(null);
    try {
      const written = await api.saveExtension(kind, draft.files);
      notify(
        "success",
        `Đã lưu ${meta.label}`,
        written.length === 1 ? written[0] : `${written.length} file đã ghi`,
      );
      await onSaved();
      onClose();
    } catch (saveError) {
      setError(api.errorMessage(saveError));
    } finally {
      setSaving(false);
    }
  };

  const file = draft?.files[activeFile];

  return (
    <div className="create-overlay" role="presentation" onMouseDown={onClose}>
      <div
        className="create-box"
        role="dialog"
        aria-modal="true"
        aria-label={`Tạo ${meta.label} bằng AI`}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="create-head">
          <div className="create-title">
            <span className="create-icon">
              <IconSparkle size={17} />
            </span>
            <div>
              <h2>Tạo {meta.label} bằng AI</h2>
              <p>{meta.hint}</p>
            </div>
          </div>
          <button className="icon-button" onClick={onClose} aria-label="Đóng" disabled={saving}>
            <IconX size={16} />
          </button>
        </header>

        <div className="create-body">
          <label className="field field-wide">
            <span>Bạn muốn tạo gì?</span>
            <textarea
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder={meta.placeholder}
              rows={4}
              disabled={loading || saving}
              autoFocus
            />
          </label>

          <div className="create-actions">
            <button
              className="btn btn-primary"
              onClick={() => void generate()}
              disabled={!prompt.trim() || loading || saving}
            >
              <IconSparkle size={15} />
              {loading ? "Đang tạo…" : "Tạo bản nháp"}
            </button>
            {draft && (
              <button className="btn" onClick={() => void save()} disabled={saving}>
                {saving ? "Đang lưu…" : "Lưu vào máy"}
              </button>
            )}
          </div>

          {loading && <Spinner label="AI đang viết extension…" />}

          {error && (
            <div className="create-error" role="alert">
              {error}
            </div>
          )}

          {draft && (
            <div className="create-preview">
              <div className="create-files">
                {draft.files.map((item, index) => (
                  <button
                    key={`${item.path}-${index}`}
                    className={`create-file ${index === activeFile ? "is-active" : ""}`}
                    onClick={() => setActiveFile(index)}
                  >
                    <code>{item.path}</code>
                  </button>
                ))}
              </div>
              {file && (
                <pre className="create-code">
                  <code>{file.content}</code>
                </pre>
              )}
              <p className="create-note">{draft.notes}</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
