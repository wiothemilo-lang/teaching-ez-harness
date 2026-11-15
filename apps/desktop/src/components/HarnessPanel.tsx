import { useCallback, useEffect, useState } from "react";
import * as api from "../api";
import type {
  HarnessConfig,
  HarnessInfo,
  HookConfig,
  MemoryRecord,
  PermissionMode,
  PermissionRule,
  ToastKind,
} from "../types";
import { IconPlus, IconRefresh, IconTrash } from "../icons";
import { Callout, EmptyNote, Spinner } from "./ui";

type Props = {
  notify: (kind: ToastKind, title: string, detail?: string) => void;
  onChanged: () => void | Promise<void>;
};

const PERMISSION_OPTIONS: Array<{
  id: PermissionMode;
  label: string;
  description: string;
}> = [
  {
    id: "read_only",
    label: "Chỉ đọc",
    description: "Chặn mọi thao tác ghi file và chạy lệnh.",
  },
  {
    id: "strict",
    label: "Nghiêm ngặt",
    description: "Hỏi trước mỗi lần ghi file hoặc chạy lệnh.",
  },
  {
    id: "standard",
    label: "Tiêu chuẩn",
    description: "Hỏi với lệnh rủi ro, cho qua thao tác thường.",
  },
  {
    id: "trust",
    label: "Tin cậy",
    description: "Chỉ chặn lệnh phá hủy rõ ràng, không hỏi.",
  },
];

const HOOK_EVENTS = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"] as const;
type HookEventName = (typeof HOOK_EVENTS)[number];

const MEMORY_KINDS: Record<string, string> = {
  user: "sở thích",
  feedback: "cách làm",
  project: "dự án",
  reference: "tra cứu",
};

type HookDraft = {
  event: HookEventName;
  command: string;
  timeout_secs: number;
};

function formatRules(rules: PermissionRule[]): string {
  return rules.map((rule) => `${rule.action} ${rule.tool} ${rule.pattern}`.trim()).join("\n");
}

function parseRules(text: string): PermissionRule[] {
  const actions = new Set(["allow", "ask", "deny"]);
  const rules: PermissionRule[] = [];
  for (const raw of text.split("\n")) {
    const parts = raw.trim().split(/\s+/).filter(Boolean);
    if (parts.length < 2) continue;
    const action = parts[0].toLowerCase();
    if (!actions.has(action)) continue;
    rules.push({
      action: action as PermissionRule["action"],
      tool: parts[1],
      pattern: parts.slice(2).join(" ") || "*",
    });
  }
  return rules;
}

/** Tab cấu hình harness: quyền, giới hạn agent, compaction, memory và hook. */
export function HarnessPanel({ notify, onChanged }: Props) {
  const [info, setInfo] = useState<HarnessInfo | null>(null);
  const [draft, setDraft] = useState<HarnessConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [hookDraft, setHookDraft] = useState<HookDraft>({
    event: "PreToolUse",
    command: "",
    timeout_secs: 5,
  });
  const [toolsText, setToolsText] = useState("");
  const [rulesText, setRulesText] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await api.getHarness();
      setInfo(next);
      setDraft(next.config);
      setToolsText(next.config.disabled_tools.join(", "));
      setRulesText(formatRules(next.config.permission_rules));
    } catch (error) {
      notify("error", "Không tải được cấu hình harness", api.errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const patch = (changes: Partial<HarnessConfig>) => {
    setDraft((current) => (current ? { ...current, ...changes } : current));
  };

  const patchCompaction = (changes: Partial<HarnessConfig["compaction"]>) => {
    setDraft((current) =>
      current ? { ...current, compaction: { ...current.compaction, ...changes } } : current,
    );
  };

  const patchMemory = (changes: Partial<HarnessConfig["memory"]>) => {
    setDraft((current) =>
      current ? { ...current, memory: { ...current.memory, ...changes } } : current,
    );
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const next = await api.saveHarness(draft);
      setInfo(next);
      setDraft(next.config);
      setToolsText(next.config.disabled_tools.join(", "));
      setRulesText(formatRules(next.config.permission_rules));
      notify("success", "Đã lưu cài đặt agent", "Cấu hình áp dụng từ lượt chat tiếp theo.");
      await onChanged();
    } catch (error) {
      notify("error", "Không lưu được harness", api.errorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  const setHooks = (hooks: HookConfig[]) => patch({ hooks });

  const updateHook = (index: number, changes: Partial<HookConfig>) => {
    if (!draft) return;
    setHooks(draft.hooks.map((hook, at) => (at === index ? { ...hook, ...changes } : hook)));
  };

  const removeHook = (index: number) => {
    if (!draft) return;
    setHooks(draft.hooks.filter((_, at) => at !== index));
  };

  const addHook = () => {
    if (!draft || !hookDraft.command.trim()) return;
    setHooks([
      ...draft.hooks,
      {
        event: hookDraft.event,
        command: hookDraft.command.trim(),
        timeout_secs: hookDraft.timeout_secs,
        enabled: true,
      },
    ]);
    setHookDraft((current) => ({ ...current, command: "" }));
  };

  const removeMemory = async (record: MemoryRecord) => {
    try {
      await api.deleteMemory(record.name);
      setInfo((current) =>
        current
          ? { ...current, memory: current.memory.filter((item) => item.name !== record.name) }
          : current,
      );
      notify("success", "Đã xoá memory", record.name);
      await onChanged();
    } catch (error) {
      notify("error", "Không xoá được memory", api.errorMessage(error));
    }
  };

  return (
    <section className="settings-section harness-section">
      <div className="section-head">
        <div>
          <h3>Agent</h3>
          <p>
            Cấu hình quyền, model, context, bộ nhớ và hành vi của agent. Thay đổi có hiệu lực từ
            lượt chat tiếp theo.
          </p>
        </div>
        <div className="head-actions">
          <button type="button" className="btn btn-sm" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} />
            Tải lại
          </button>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={() => void save()}
            disabled={saving || !draft}
          >
            {saving ? "Đang lưu…" : "Lưu"}
          </button>
        </div>
      </div>

      {loading && !draft ? (
        <Spinner label="Đang tải harness…" />
      ) : !draft ? (
        <EmptyNote>Chưa đọc được cấu hình harness.</EmptyNote>
      ) : (
        <>
          <div className="harness-block">
            <div className="harness-block-head">
              <h4>Quyền thao tác</h4>
              <p>Deny-list lệnh phá hủy luôn được áp dụng, kể cả ở chế độ tin cậy.</p>
            </div>
            <div className="mode-grid" role="radiogroup" aria-label="Permission mode">
              {PERMISSION_OPTIONS.map((option) => (
                <label
                  key={option.id}
                  className={`mode-option ${
                    draft.permission_mode === option.id ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="permission-mode"
                    value={option.id}
                    checked={draft.permission_mode === option.id}
                    onChange={() => patch({ permission_mode: option.id })}
                  />
                  <span className="mode-copy">
                    <strong>{option.label}</strong>
                    <small>{option.description}</small>
                  </span>
                </label>
              ))}
            </div>
            <label className="field field-wide">
              <span>Permission rules</span>
              <textarea
                className="system-prompt rules-input"
                value={rulesText}
                onChange={(event) => {
                  const value = event.target.value;
                  setRulesText(value);
                  patch({ permission_rules: parseRules(value) });
                }}
                placeholder={"allow write_file src/**\nask run_command git push*\ndeny write_file secrets/**"}
                rows={4}
                spellCheck={false}
              />
            </label>
            <p className="field-note">
              Mỗi dòng: <code>allow|ask|deny tool pattern</code>. Pattern là glob cho path hoặc
              command; <code>*</code> khớp mọi đối tượng. Deny-list lệnh phá hủy vẫn luôn thắng
              rule.
            </p>
          </div>

          <div className="harness-block">
            <div className="harness-block-head">
              <h4>Giới hạn agent</h4>
              <p>Chặn vòng lặp chạy quá lâu hoặc trả lời quá dài.</p>
            </div>
            <div className="field-grid">
              <label className="field">
                <span>Số bước tối đa</span>
                <input
                  type="number"
                  min={1}
                  max={100}
                  value={draft.max_steps}
                  onChange={(event) => patch({ max_steps: Number(event.target.value) || 1 })}
                />
              </label>
              <label className="field">
                <span>Max tokens / lượt</span>
                <input
                  type="number"
                  min={128}
                  max={32768}
                  step={128}
                  value={draft.max_tokens}
                  onChange={(event) => patch({ max_tokens: Number(event.target.value) || 128 })}
                />
              </label>
              <label className="field">
                <span>Temperature</span>
                <input
                  type="number"
                  min={0}
                  max={2}
                  step={0.1}
                  value={draft.temperature}
                  onChange={(event) => patch({ temperature: Number(event.target.value) || 0 })}
                />
              </label>
              <label className="field">
                <span>Top P</span>
                <input
                  type="number"
                  min={0}
                  max={1}
                  step={0.05}
                  value={draft.top_p}
                  onChange={(event) => patch({ top_p: Number(event.target.value) || 0 })}
                />
              </label>
              <label className="field">
                <span>Output tool tối đa (byte)</span>
                <input
                  type="number"
                  min={1024}
                  max={65536}
                  step={1024}
                  value={draft.max_tool_output}
                  onChange={(event) =>
                    patch({ max_tool_output: Number(event.target.value) || 1024 })
                  }
                />
              </label>
              <label className="field">
                <span>Số bước subagent</span>
                <input
                  type="number"
                  min={1}
                  max={50}
                  value={draft.subagent_max_steps}
                  onChange={(event) =>
                    patch({ subagent_max_steps: Number(event.target.value) || 1 })
                  }
                />
              </label>
            </div>
          </div>

          <div className="harness-block">
            <div className="harness-block-head">
              <h4>Hành vi & ngôn ngữ</h4>
              <p>
                Chỉ dẫn bổ sung được nối vào system prompt. Dùng cho phong cách trả lời, quy ước
                riêng hoặc ngôn ngữ mặc định.
              </p>
            </div>
            <label className="field field-wide">
              <span>System prompt bổ sung</span>
              <textarea
                className="system-prompt"
                value={draft.system_prompt}
                onChange={(event) => patch({ system_prompt: event.target.value })}
                placeholder="Ví dụ: trả lời ngắn gọn, luôn nêu rủi ro trước khi sửa file, dùng tiếng Việt…"
                rows={4}
              />
            </label>
            <div className="field-grid">
              <label className="field">
                <span>Ngôn ngữ trả lời</span>
                <select
                  value={draft.language}
                  onChange={(event) => patch({ language: event.target.value })}
                >
                  <option value="vi">Tiếng Việt</option>
                  <option value="en">English</option>
                </select>
              </label>
              <label className="field field-wide">
                <span>Tool bị tắt</span>
                <input
                  value={toolsText}
                  onChange={(event) => {
                    const value = event.target.value;
                    setToolsText(value);
                    patch({
                      disabled_tools: value
                        .split(",")
                        .map((item) => item.trim())
                        .filter(Boolean),
                    });
                  }}
                  placeholder="ví dụ: run_command, task"
                  spellCheck={false}
                />
              </label>
            </div>
            <p className="field-note">
              Tên tool phân tách bằng dấu phẩy. Tool bị tắt sẽ không được gửi cho model ở lượt
              tiếp theo.
            </p>
          </div>

          <div className="harness-block">
            <div className="harness-block-head">
              <h4>Nén context</h4>
              <p>
                Bốn bước từ rẻ tới đắt: lưu kết quả lớn, cắt lịch sử cũ, micro-compact, rồi mới
                tóm tắt bằng model.
              </p>
            </div>
            <label className="checkbox harness-toggle">
              <input
                type="checkbox"
                checked={draft.compaction.enabled}
                onChange={(event) => patchCompaction({ enabled: event.target.checked })}
              />
              Bật nén context tự động
            </label>
            <div className="field-grid">
              <label className="field">
                <span>Giới hạn ký tự</span>
                <input
                  type="number"
                  min={8000}
                  max={500000}
                  step={1000}
                  value={draft.compaction.context_char_limit}
                  onChange={(event) =>
                    patchCompaction({ context_char_limit: Number(event.target.value) || 8000 })
                  }
                />
              </label>
              <label className="field">
                <span>Kết quả lớn (ký tự)</span>
                <input
                  type="number"
                  min={2000}
                  max={200000}
                  step={1000}
                  value={draft.compaction.large_result_chars}
                  onChange={(event) =>
                    patchCompaction({ large_result_chars: Number(event.target.value) || 2000 })
                  }
                />
              </label>
              <label className="field">
                <span>Preview (ký tự)</span>
                <input
                  type="number"
                  min={200}
                  max={10000}
                  step={100}
                  value={draft.compaction.preview_chars}
                  onChange={(event) =>
                    patchCompaction({ preview_chars: Number(event.target.value) || 200 })
                  }
                />
              </label>
              <label className="field">
                <span>Trigger (số tin)</span>
                <input
                  type="number"
                  min={10}
                  max={500}
                  value={draft.compaction.trigger_messages}
                  onChange={(event) =>
                    patchCompaction({ trigger_messages: Number(event.target.value) || 10 })
                  }
                />
              </label>
              <label className="field">
                <span>Giữ lại (số tin)</span>
                <input
                  type="number"
                  min={5}
                  max={499}
                  value={draft.compaction.target_messages}
                  onChange={(event) =>
                    patchCompaction({ target_messages: Number(event.target.value) || 5 })
                  }
                />
              </label>
              <label className="field">
                <span>Giữ kết quả gần nhất</span>
                <input
                  type="number"
                  min={0}
                  max={20}
                  value={draft.compaction.keep_recent_results}
                  onChange={(event) =>
                    patchCompaction({ keep_recent_results: Number(event.target.value) || 0 })
                  }
                />
              </label>
            </div>
          </div>

          <div className="harness-block">
            <div className="harness-block-head">
              <h4>Bộ nhớ dài hạn</h4>
              <p>Mỗi bản ghi là một file Markdown; chỉ bản ghi liên quan được nạp vào context.</p>
            </div>
            <div className="harness-toggle-row">
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.memory.enabled}
                  onChange={(event) => patchMemory({ enabled: event.target.checked })}
                />
                Bật bộ nhớ
              </label>
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.memory.auto_recall}
                  onChange={(event) => patchMemory({ auto_recall: event.target.checked })}
                />
                Tự động nạp bản ghi liên quan
              </label>
              <label className="field field-inline">
                <span>Tối đa</span>
                <input
                  type="number"
                  min={0}
                  max={10}
                  value={draft.memory.recall_limit}
                  onChange={(event) =>
                    patchMemory({ recall_limit: Number(event.target.value) || 0 })
                  }
                />
              </label>
            </div>

            {info && info.memory.length > 0 ? (
              <div className="memory-list">
                {info.memory.map((record) => (
                  <div className="memory-row" key={record.path}>
                    <div className="memory-copy">
                      <div className="memory-head">
                        <strong>{record.name}</strong>
                        <span className="memory-kind">
                          {MEMORY_KINDS[record.kind] ?? record.kind}
                        </span>
                      </div>
                      <p>{record.description || record.body.slice(0, 160)}</p>
                    </div>
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      title={`Xoá ${record.name}`}
                      onClick={() => void removeMemory(record)}
                    >
                      <IconTrash size={13} />
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyNote>Chưa có bản ghi. Agent có thể lưu bằng tool memory_save.</EmptyNote>
            )}
          </div>

          <div className="harness-block">
            <div className="harness-block-head">
              <h4>Hook</h4>
              <p>
                Lệnh shell nhận JSON qua stdin và trả JSON qua stdout. Hook lỗi hoặc chặn sẽ dừng
                tool tương ứng.
              </p>
            </div>
            {draft.hooks.length > 0 ? (
              <div className="hook-list">
                {draft.hooks.map((hook, index) => (
                  <div className="hook-row" key={`${hook.event}-${index}`}>
                    <select
                      value={hook.event}
                      onChange={(event) => updateHook(index, { event: event.target.value })}
                      aria-label="Hook event"
                    >
                      {HOOK_EVENTS.map((name) => (
                        <option key={name} value={name}>
                          {name}
                        </option>
                      ))}
                    </select>
                    <input
                      value={hook.command}
                      onChange={(event) => updateHook(index, { command: event.target.value })}
                      aria-label="Hook command"
                      placeholder="lệnh shell"
                    />
                    <input
                      type="number"
                      min={1}
                      max={60}
                      value={hook.timeout_secs}
                      onChange={(event) =>
                        updateHook(index, { timeout_secs: Number(event.target.value) || 1 })
                      }
                      aria-label="Hook timeout"
                      title="Timeout (giây)"
                    />
                    <label className="checkbox hook-enabled" title="Bật hook">
                      <input
                        type="checkbox"
                        checked={hook.enabled}
                        onChange={(event) => updateHook(index, { enabled: event.target.checked })}
                      />
                    </label>
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      title="Xoá hook"
                      onClick={() => removeHook(index)}
                    >
                      <IconTrash size={13} />
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyNote>Chưa có hook tùy chỉnh. Audit log và che bí mật luôn bật.</EmptyNote>
            )}

            <div className="hook-row hook-add">
              <select
                value={hookDraft.event}
                onChange={(event) =>
                  setHookDraft((current) => ({
                    ...current,
                    event: event.target.value as HookEventName,
                  }))
                }
                aria-label="Hook event mới"
              >
                {HOOK_EVENTS.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
              <input
                value={hookDraft.command}
                onChange={(event) =>
                  setHookDraft((current) => ({ ...current, command: event.target.value }))
                }
                placeholder="ví dụ: node scripts/check.js"
                aria-label="Hook command mới"
                onKeyDown={(event) => {
                  if (event.key === "Enter") addHook();
                }}
              />
              <input
                type="number"
                min={1}
                max={60}
                value={hookDraft.timeout_secs}
                onChange={(event) =>
                  setHookDraft((current) => ({
                    ...current,
                    timeout_secs: Number(event.target.value) || 1,
                  }))
                }
                aria-label="Hook timeout mới"
                title="Timeout (giây)"
              />
              <button
                type="button"
                className="btn btn-sm"
                onClick={addHook}
                disabled={!hookDraft.command.trim()}
              >
                <IconPlus size={13} />
                Thêm
              </button>
            </div>
          </div>

          {info && (
            <Callout>
              <div className="harness-paths">
                <span>
                  Cấu hình: <code>{info.path}</code>
                </span>
                <span>
                  Bộ nhớ: <code>{info.memory_dir}</code>
                </span>
                <span>
                  Audit: <code>{info.audit_log}</code>
                </span>
              </div>
            </Callout>
          )}
        </>
      )}
    </section>
  );
}
