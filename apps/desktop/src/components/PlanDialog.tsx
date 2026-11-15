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
              <label className="field field-wide">
                <span>Nội dung kế hoạch</span>
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
