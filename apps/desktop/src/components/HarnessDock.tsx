import type { CompactReport, TodoItem } from "../types";
import { IconCheck, IconLock, IconSliders } from "../icons";

type Props = {
  todos: TodoItem[];
  compaction: CompactReport | null;
  contextLimit: number;
  denials: number;
  onOpenSettings: () => void;
};

function formatChars(value: number): string {
  if (value >= 1_000) return `${Math.round(value / 1_000)}k`;
  return String(value);
}

/** Thanh trạng thái harness: kế hoạch, mức context và số thao tác bị chặn. */
export function HarnessDock({ todos, compaction, contextLimit, denials, onOpenSettings }: Props) {
  const hasTodos = todos.length > 0;
  const hasContext = Boolean(compaction);
  if (!hasTodos && !hasContext && denials === 0) return null;

  const done = todos.filter((item) => item.status === "completed").length;
  const estimate = compaction?.estimated_chars ?? 0;
  const percent =
    contextLimit > 0 ? Math.min(100, Math.round((estimate / contextLimit) * 100)) : 0;
  const level = percent >= 90 ? "danger" : percent >= 70 ? "warn" : "ok";
  const compacted =
    (compaction?.persisted ?? 0) +
    (compaction?.snipped ?? 0) +
    (compaction?.micro_compacted ?? 0);

  return (
    <section className="harness-dock" aria-label="Trạng thái harness">
      {hasTodos && (
        <div className="plan-panel">
          <div className="plan-head">
            <span className="plan-title">Kế hoạch</span>
            <span className="plan-count">
              {done}/{todos.length}
            </span>
          </div>
          <ol className="plan-list">
            {todos.map((item, index) => (
              <li
                key={`${index}-${item.content}`}
                className={`plan-item is-${item.status}`}
                title={item.content}
              >
                <span className="plan-mark" aria-hidden="true">
                  {item.status === "completed" ? (
                    <IconCheck size={11} />
                  ) : item.status === "in_progress" ? (
                    <span className="plan-dot" />
                  ) : null}
                </span>
                <span className="plan-text">{item.content}</span>
              </li>
            ))}
          </ol>
        </div>
      )}

      <div className="harness-meta">
        {hasContext && (
          <div className={`context-meter is-${level}`}>
            <div className="context-line">
              <span>Context</span>
              <span className="context-value">
                {formatChars(estimate)} / {formatChars(contextLimit)}
              </span>
            </div>
            <div className="context-track" aria-hidden="true">
              <span style={{ width: `${percent}%` }} />
            </div>
            <div className="context-tags">
              {compaction?.summarized && <span className="context-tag">đã tóm tắt</span>}
              {compacted > 0 && <span className="context-tag">{compacted} lượt nén</span>}
              <span className="context-tag">{percent}%</span>
            </div>
          </div>
        )}

        {denials > 0 && (
          <span className="harness-denial">
            <IconLock size={12} />
            {denials} thao tác bị chặn
          </span>
        )}

        <button
          type="button"
          className="btn btn-sm btn-ghost harness-settings"
          onClick={onOpenSettings}
        >
          <IconSliders size={14} />
          Harness
        </button>
      </div>
    </section>
  );
}
