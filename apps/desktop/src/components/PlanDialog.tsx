import { useEffect, useState } from "react";
import * as api from "../api";
import type { ToastKind } from "../types";
import { IconBook, IconX } from "../icons";
import { Spinner } from "./ui";

type Props = {
  open: boolean;
  sessionId: string | null;
  onClose: () => void;
  notify: (kind: ToastKind, title: string, detail?: string) => void;
};

const PLAN_SECTIONS = [
  "Mục tiêu",
  "Phạm vi",
  "File và thay đổi",
  "Các bước",
  "Rủi ro",
  "Kiểm chứng",
];

function normalizeTitle(value: string): string {
  return value.trim().replace(/:$/, "").trim().toLowerCase();
}

function parseSections(text: string): Record<string, string> {
  const values: Record<string, string> = {};
  let current: string | null = null;
  let lines: string[] = [];

  const flush = () => {
    if (current) values[current] = lines.join("\n").trim();
  };

  for (const line of text.split(/\r?\n/)) {
    const heading = line.match(/^##\s+(.+?)\s*$/);
    if (heading) {
      flush();
      current = normalizeTitle(heading[1]);
      lines = [];
    } else if (current) {
      lines.push(line);
    }
  }
  flush();
  return values;
}

function upsertSection(text: string, title: string, content: string): string {
  const lines = text.split(/\r?\n/);
  const wanted = normalizeTitle(title);
  let start = -1;
  for (let index = 0; index < lines.length; index += 1) {
    const heading = lines[index].match(/^##\s+(.+?)\s*$/);
    if (heading && normalizeTitle(heading[1]) === wanted) {
      start = index;
      break;
    }
  }

  const body = content.trim() ? content.trim().split(/\r?\n/) : [];
  if (start === -1) {
    const prefix = text.trim() ? `${text.trimEnd()}\n\n` : "";
    return `${prefix}## ${title}\n${body.join("\n")}${body.length > 0 ? "\n" : ""}`;
  }

  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^##\s+/.test(lines[index])) {
      end = index;
      break;
    }
  }

  return [...lines.slice(0, start + 1), ...body, ...lines.slice(end)].join("\n");
}

export function PlanDialog({ open, sessionId, onClose, notify }: Props) {
  const [text, setText] = useState("");
  const [path, setPath] = useState("");
  const [valid, setValid] = useState(true);
  const [missing, setMissing] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    if (!sessionId) {
      setText("");
      setPath("");
      setValid(true);
      setMissing([]);
      return;
    }
    let alive = true;
    setLoading(true);
    api
      .loadSessionPlan(sessionId)
      .then((info) => {
        if (!alive) return;
        setText(info.text);
        setPath(info.path);
        setValid(info.valid);
        setMissing(info.missing);
      })
      .catch((loadError) => {
        if (alive) setError(api.errorMessage(loadError));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [open, sessionId]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && !saving) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, saving, onClose]);

  if (!open) return null;

  const sections = parseSections(text);

  const save = async () => {
    if (!sessionId || saving) return;
    setSaving(true);
    setError(null);
    try {
      const info = await api.saveSessionPlan(sessionId, text);
      setPath(info.path);
      setValid(info.valid);
      setMissing(info.missing);
      notify("success", "Đã lưu kế hoạch", info.path);
    } catch (saveError) {
      setError(api.errorMessage(saveError));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="create-overlay" role="presentation" onMouseDown={onClose}>
      <div
        className="create-box plan-box"
        role="dialog"
        aria-modal="true"
        aria-label="Kế hoạch của phiên"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="create-head">
          <div className="create-title">
            <span className="create-icon">
              <IconBook size={17} />
            </span>
            <div>
              <h2>Kế hoạch của phiên</h2>
              <p>{path || "Chưa có file kế hoạch cho phiên này"}</p>
            </div>
          </div>
          <button className="icon-button" onClick={onClose} aria-label="Đóng" disabled={saving}>
            <IconX size={16} />
          </button>
        </header>

        <div className="create-body">
          {!sessionId ? (
            <p className="field-note">Bắt đầu một phiên Plan để tạo kế hoạch đầu tiên.</p>
          ) : loading ? (
            <Spinner label="Đang tải kế hoạch…" />
          ) : (
            <>
              <div className="plan-form">
                <div className="plan-form-head">
                  <strong>Sửa theo từng mục</strong>
                  <p>
                    Form quản lý 6 mục bắt buộc; các mục khác trong Markdown vẫn được giữ nguyên.
                  </p>
                </div>
                {PLAN_SECTIONS.map((title) => (
                  <label className="field field-wide plan-section-field" key={title}>
                    <span>{title}</span>
                    <textarea
                      className="plan-section"
                      rows={
                        title === "Các bước" || title === "File và thay đổi" ? 4 : 2
                      }
                      value={sections[normalizeTitle(title)] ?? ""}
                      onChange={(event) =>
                        setText(upsertSection(text, title, event.target.value))
                      }
                      disabled={saving}
                    />
                  </label>
                ))}
              </div>

              <label className="field field-wide">
                <span>Markdown đầy đủ</span>
                <textarea
                  className="plan-text"
                  value={text}
                  onChange={(event) => setText(event.target.value)}
                  rows={16}
                  placeholder="## Mục tiêu&#10;## Phạm vi&#10;## File và thay đổi&#10;## Các bước&#10;## Rủi ro&#10;## Kiểm chứng"
                  disabled={saving}
                />
              </label>

              {text.trim() && !valid && (
                <div className="plan-warning" role="status">
                  <strong>Kế hoạch thiếu mục:</strong> {missing.join(", ")}
                </div>
              )}

              {error && (
                <div className="create-error" role="alert">
                  {error}
                </div>
              )}

              <div className="create-actions">
                <button className="btn" onClick={onClose} disabled={saving}>
                  Đóng
                </button>
                <button
                  className="btn btn-primary"
                  onClick={() => void save()}
                  disabled={saving || !text.trim()}
                >
                  {saving ? "Đang lưu…" : "Lưu kế hoạch"}
                </button>
              </div>

              <p className="field-note">
                File nằm ngoài SQLite tại đường dẫn trên. Agent ở chế độ Code sẽ đọc lại kế hoạch
                này ở lượt tiếp theo.
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
