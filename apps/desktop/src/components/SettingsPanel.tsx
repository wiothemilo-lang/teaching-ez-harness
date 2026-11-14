import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import * as api from "../api";
import type {
  AppStatus,
  ExtensionsInfo,
  McpServerInfo,
  McpTest,
  ProviderInfo,
  SettingsTab,
  ToastKind,
} from "../types";
import { PROVIDER_LABELS, parseEnv, splitArgs, subjectLabel, truncate } from "../lib";
import {
  IconBook,
  IconChevronDown,
  IconDatabase,
  IconDownload,
  IconEye,
  IconEyeOff,
  IconFolder,
  IconKey,
  IconPen,
  IconPlug,
  IconPlus,
  IconPuzzle,
  IconRefresh,
  IconTerminal,
  IconTrash,
  IconX,
} from "../icons";
import { Callout, EmptyNote, Spinner, Tag } from "./ui";
import { UpdatePanel } from "./UpdatePanel";

type Props = {
  open: boolean;
  tab: SettingsTab;
  status: AppStatus | null;
  onTab: (tab: SettingsTab) => void;
  onClose: () => void;
  onChanged: () => void | Promise<void>;
  notify: (kind: ToastKind, title: string, detail?: string) => void;
};

type Preset = {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  keyHint: string;
  note: string;
};

type ProviderDraft = {
  id?: string;
  name: string;
  provider: string;
  base_url: string;
  model: string;
  api_key: string;
  set_active: boolean;
};

type McpDraft = {
  originalName?: string;
  name: string;
  command: string;
  argsText: string;
  cwd: string;
  envText: string;
  enabled: boolean;
};

const PRESETS: Preset[] = [
  {
    id: "groq",
    name: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    model: "llama-3.3-70b-versatile",
    keyHint: "gsk_…",
    note: "Free tier, phản hồi rất nhanh",
  },
  {
    id: "gemini",
    name: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    model: "gemini-2.0-flash",
    keyHint: "AIza…",
    note: "Free tier, context dài",
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    model: "meta-llama/llama-3.3-70b-instruct",
    keyHint: "sk-or-…",
    note: "Nhiều model, có model miễn phí",
  },
  {
    id: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    keyHint: "sk-…",
    note: "Trả phí theo token",
  },
  {
    id: "ollama",
    name: "Ollama",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "qwen3:4b",
    keyHint: "không cần key",
    note: "Chạy hoàn toàn trên máy",
  },
  {
    id: "custom",
    name: "Custom",
    baseUrl: "",
    model: "",
    keyHint: "tùy endpoint",
    note: "Mọi endpoint OpenAI-compatible",
  },
];

const TAB_META: { id: SettingsTab; label: string; icon: ReactNode }[] = [
  { id: "providers", label: "Nhà cung cấp", icon: <IconKey size={16} /> },
  { id: "skills", label: "Kỹ năng", icon: <IconBook size={16} /> },
  { id: "mcp", label: "MCP", icon: <IconPlug size={16} /> },
  { id: "plugins", label: "Plugin", icon: <IconPuzzle size={16} /> },
  { id: "update", label: "Cập nhật", icon: <IconDownload size={16} /> },
  { id: "data", label: "Dữ liệu", icon: <IconDatabase size={16} /> },
];

const BUILTIN_SKILL_IDS = new Set([
  "step-by-step",
  "error-diagnosis",
  "exam-review",
  "english-writing",
]);

function keyPlaceholder(provider: string): string {
  if (provider === "ollama") return "Không cần API key";
  if (provider === "custom") return "Để trống nếu endpoint không cần key";
  return "Dán API key vào đây…";
}

export function SettingsPanel({ open, tab, status, onTab, onClose, onChanged, notify }: Props) {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [extensions, setExtensions] = useState<ExtensionsInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [draft, setDraft] = useState<ProviderDraft | null>(null);
  const [saving, setSaving] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [expandedSkill, setExpandedSkill] = useState<string | null>(null);
  const [mcpDraft, setMcpDraft] = useState<McpDraft | null>(null);
  const [mcpSaving, setMcpSaving] = useState(false);
  const [mcpTests, setMcpTests] = useState<Record<string, McpTest | "loading">>({});
  const [confirmMcpDelete, setConfirmMcpDelete] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [nextProviders, nextExtensions] = await Promise.all([
        api.listProviders(),
        api.listExtensions(),
      ]);
      setProviders(nextProviders);
      setExtensions(nextExtensions);
    } catch (error) {
      notify("error", "Không tải được cài đặt", api.errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [notify]);

  useEffect(() => {
    if (open) void reload();
  }, [open, reload]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    panelRef.current?.focus();
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = previous;
    };
  }, [open, onClose]);

  if (!open) return null;

  const counts: Record<SettingsTab, number | null> = {
    providers: providers.length,
    skills: extensions?.skills?.length ?? null,
    mcp: extensions?.mcp?.length ?? null,
    plugins: extensions?.plugins?.length ?? null,
    update: null,
    data: null,
  };

  const updateDraft = (patch: Partial<ProviderDraft>) => {
    setDraft((previous) => (previous ? { ...previous, ...patch } : previous));
  };

  const startAddProvider = () => {
    const preset = PRESETS[0];
    setShowKey(false);
    setConfirmDelete(null);
    setDraft({
      name: preset.name,
      provider: preset.id,
      base_url: preset.baseUrl,
      model: preset.model,
      api_key: "",
      set_active: true,
    });
  };

  const startEditProvider = (provider: ProviderInfo) => {
    setShowKey(false);
    setConfirmDelete(null);
    setDraft({
      id: provider.id,
      name: provider.name,
      provider: provider.provider,
      base_url: provider.base_url,
      model: provider.model,
      api_key: "",
      set_active: provider.active,
    });
  };

  const applyPreset = (preset: Preset) => {
    updateDraft({
      provider: preset.id,
      name: preset.name,
      base_url: preset.baseUrl,
      model: preset.model,
    });
  };

  const saveProvider = async () => {
    if (!draft) return;
    const name = draft.name.trim();
    const baseUrl = draft.base_url.trim();
    const model = draft.model.trim();
    if (!name) {
      notify("error", "Thiếu tên gợi nhớ", "Đặt tên để phân biệt các nhà cung cấp.");
      return;
    }
    if (!baseUrl) {
      notify("error", "Thiếu base URL", "Endpoint OpenAI-compatible cần có địa chỉ.");
      return;
    }
    if (!model) {
      notify("error", "Thiếu model", "Nhập model id mà endpoint hỗ trợ.");
      return;
    }

    setSaving(true);
    try {
      await api.saveProvider({
        id: draft.id ?? null,
        name,
        provider: draft.provider,
        base_url: baseUrl,
        model,
        api_key: draft.id && draft.api_key === "" ? null : draft.api_key,
        set_active: draft.set_active,
      });
      setDraft(null);
      await reload();
      await onChanged();
      notify("success", draft.id ? "Đã cập nhật nhà cung cấp" : "Đã thêm nhà cung cấp", `${name} · ${model}`);
    } catch (error) {
      notify("error", "Không lưu được nhà cung cấp", api.errorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  const activateProvider = async (provider: ProviderInfo) => {
    try {
      await api.activateProvider(provider.id);
      await reload();
      await onChanged();
      notify("success", "Đã chuyển nhà cung cấp", `${provider.name} · ${provider.model}`);
    } catch (error) {
      notify("error", "Không chuyển được nhà cung cấp", api.errorMessage(error));
    }
  };

  const testProvider = async (provider: ProviderInfo) => {
    setTestingId(provider.id);
    try {
      const result = await api.testProvider(provider.id);
      notify(
        result.ok ? "success" : "error",
        `Kiểm tra ${provider.name}`,
        `${result.latency_ms} ms · ${truncate(result.message, 90)}`,
      );
    } catch (error) {
      notify("error", `Không kết nối được ${provider.name}`, api.errorMessage(error));
    } finally {
      setTestingId(null);
    }
  };

  const removeProvider = async (provider: ProviderInfo) => {
    try {
      await api.deleteProvider(provider.id);
      setConfirmDelete(null);
      if (draft?.id === provider.id) setDraft(null);
      await reload();
      await onChanged();
      notify("success", "Đã xóa nhà cung cấp", provider.name);
    } catch (error) {
      notify("error", "Không xóa được nhà cung cấp", api.errorMessage(error));
    }
  };

  const startAddMcp = () => {
    setConfirmMcpDelete(null);
    setMcpDraft({
      name: "",
      command: "",
      argsText: "",
      cwd: "",
      envText: "",
      enabled: true,
    });
  };

  const startEditMcp = (server: McpServerInfo) => {
    setConfirmMcpDelete(null);
    setMcpDraft({
      originalName: server.name,
      name: server.name,
      command: server.command,
      argsText: server.args.join(" "),
      cwd: server.cwd ?? "",
      envText: Object.entries(server.env)
        .map(([key, value]) => `${key}=${value}`)
        .join("\n"),
      enabled: server.enabled,
    });
  };

  const saveMcp = async () => {
    if (!mcpDraft) return;
    const name = (mcpDraft.originalName ?? mcpDraft.name).trim();
    const command = mcpDraft.command.trim();
    if (!name) {
      notify("error", "Thiếu tên MCP server");
      return;
    }
    if (!command) {
      notify("error", "Thiếu command", "Ví dụ: python3, npx hoặc uvx.");
      return;
    }

    let env: Record<string, string>;
    try {
      env = parseEnv(mcpDraft.envText);
    } catch (error) {
      notify("error", "Env không hợp lệ", api.errorMessage(error));
      return;
    }

    setMcpSaving(true);
    try {
      await api.saveMcpServer({
        name,
        command,
        args: splitArgs(mcpDraft.argsText),
        env,
        cwd: mcpDraft.cwd.trim() || null,
        enabled: mcpDraft.enabled,
      });
      setMcpDraft(null);
      await reload();
      await onChanged();
      notify("success", "Đã lưu MCP server", name);
    } catch (error) {
      notify("error", "Không lưu được MCP server", api.errorMessage(error));
    } finally {
      setMcpSaving(false);
    }
  };

  const testMcp = async (name: string) => {
    setMcpTests((previous) => ({ ...previous, [name]: "loading" }));
    try {
      const result = await api.testMcpServer(name);
      setMcpTests((previous) => ({ ...previous, [name]: result }));
      if (!result.ok) {
        notify("error", `MCP ${name} lỗi`, result.error ?? "Không rõ lỗi");
      }
    } catch (error) {
      const message = api.errorMessage(error);
      setMcpTests((previous) => ({
        ...previous,
        [name]: { ok: false, tools: [], error: message },
      }));
      notify("error", `MCP ${name} lỗi`, message);
    }
  };

  const removeMcp = async (server: McpServerInfo) => {
    try {
      await api.deleteMcpServer(server.name);
      setConfirmMcpDelete(null);
      if (mcpDraft?.originalName === server.name) setMcpDraft(null);
      await reload();
      await onChanged();
      notify("success", "Đã xóa MCP server", server.name);
    } catch (error) {
      notify("error", "Không xóa được MCP server", api.errorMessage(error));
    }
  };

  const installSample = async () => {
    setInstalling(true);
    try {
      const plugin = await api.installSamplePlugin();
      await reload();
      await onChanged();
      notify("success", "Đã cài plugin mẫu", `${plugin.name} · ${plugin.skills} kỹ năng, ${plugin.mcp_servers.length} MCP`);
    } catch (error) {
      notify("error", "Không cài được plugin mẫu", api.errorMessage(error));
    } finally {
      setInstalling(false);
    }
  };

  const openPath = async (kind: "skills" | "plugins" | "data" | "mcp") => {
    try {
      await api.openExtensionPath(kind);
    } catch (error) {
      notify("error", "Không mở được thư mục", api.errorMessage(error));
    }
  };

  const content = () => {
    if (tab === "update") {
      return <UpdatePanel current={status?.version ?? "0.1.0"} notify={notify} />;
    }

    if (loading && !extensions) {
      return (
        <div className="settings-loading">
          <Spinner label="Đang tải cài đặt…" />
        </div>
      );
    }
    if (!extensions && !loading) {
      return (
        <div className="settings-loading">
          <p>Không tải được dữ liệu cài đặt.</p>
          <button className="btn" onClick={() => void reload()}>
            <IconRefresh size={14} />
            Thử lại
          </button>
        </div>
      );
    }

    if (tab === "providers") {
      return (
        <section className="settings-section">
          <div className="section-head">
            <div>
              <h3>Nhà cung cấp</h3>
              <p>
                Lưu nhiều tài khoản hoặc endpoint và chuyển giữa chúng. tez chỉ gửi request tới
                provider đang chọn.
              </p>
            </div>
            {!draft && (
              <button className="btn btn-primary" onClick={startAddProvider}>
                <IconPlus size={15} />
                Thêm
              </button>
            )}
          </div>

          {providers.length === 0 ? (
            <EmptyNote>
              Chưa có nhà cung cấp. Thêm Groq, Gemini hoặc một endpoint local để bắt đầu.
            </EmptyNote>
          ) : (
            <div className="provider-list">
              {providers.map((provider) => (
                <div
                  key={provider.id}
                  className={`provider-row ${provider.active ? "is-active" : ""}`}
                >
                  <div className="provider-main">
                    <div className="provider-title">
                      <span className="provider-name">{provider.name}</span>
                      {provider.active && <Tag tone="forest">Đang dùng</Tag>}
                      <Tag tone={provider.has_key ? "muted" : "neutral"}>
                        {provider.has_key ? "Key đã lưu" : "Chưa có key"}
                      </Tag>
                    </div>
                    <div className="provider-meta">
                      <span>{PROVIDER_LABELS[provider.provider] ?? provider.provider}</span>
                      <span className="dot-sep">·</span>
                      <code>{provider.model}</code>
                    </div>
                    <div className="provider-url">{provider.base_url}</div>
                  </div>
                  <div className="row-actions">
                    {!provider.active && (
                      <button className="btn btn-sm" onClick={() => void activateProvider(provider)}>
                        Dùng
                      </button>
                    )}
                    <button
                      className="btn btn-sm"
                      onClick={() => void testProvider(provider)}
                      disabled={testingId === provider.id}
                    >
                      {testingId === provider.id ? "Đang thử…" : "Kiểm tra"}
                    </button>
                    <button
                      className="btn btn-sm btn-ghost"
                      onClick={() => startEditProvider(provider)}
                    >
                      <IconPen size={14} />
                      Sửa
                    </button>
                    {confirmDelete === provider.id ? (
                      <>
                        <button
                          className="btn btn-sm btn-danger"
                          onClick={() => void removeProvider(provider)}
                        >
                          Xóa
                        </button>
                        <button
                          className="btn btn-sm btn-ghost"
                          onClick={() => setConfirmDelete(null)}
                        >
                          Hủy
                        </button>
                      </>
                    ) : (
                      <button
                        className="icon-button icon-danger"
                        aria-label={`Xóa ${provider.name}`}
                        onClick={() => setConfirmDelete(provider.id)}
                      >
                        <IconTrash size={15} />
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}

          {draft && (
            <div className="form-panel">
              <div className="form-title">
                <h4>{draft.id ? "Sửa nhà cung cấp" : "Thêm nhà cung cấp"}</h4>
                <button
                  className="icon-button"
                  onClick={() => setDraft(null)}
                  aria-label="Đóng form"
                >
                  <IconX size={16} />
                </button>
              </div>

              <div className="preset-grid">
                {PRESETS.map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    className={`preset ${draft.provider === preset.id ? "is-selected" : ""}`}
                    onClick={() => applyPreset(preset)}
                  >
                    <span className="preset-name">{preset.name}</span>
                    <span className="preset-note">{preset.note}</span>
                  </button>
                ))}
              </div>

              <div className="form-grid">
                <label className="field">
                  <span>Tên gợi nhớ</span>
                  <input
                    value={draft.name}
                    onChange={(event) => updateDraft({ name: event.target.value })}
                    placeholder="Ví dụ: Groq cá nhân"
                  />
                </label>
                <label className="field">
                  <span>Loại</span>
                  <select
                    value={draft.provider}
                    onChange={(event) => updateDraft({ provider: event.target.value })}
                  >
                    {PRESETS.map((preset) => (
                      <option key={preset.id} value={preset.id}>
                        {preset.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field field-wide">
                  <span>Base URL</span>
                  <input
                    value={draft.base_url}
                    onChange={(event) => updateDraft({ base_url: event.target.value })}
                    placeholder="https://api.example.com/v1"
                    spellCheck={false}
                  />
                </label>
                <label className="field field-wide">
                  <span>Model</span>
                  <input
                    value={draft.model}
                    onChange={(event) => updateDraft({ model: event.target.value })}
                    placeholder="model-id"
                    spellCheck={false}
                  />
                </label>
                <label className="field field-wide">
                  <span>API key</span>
                  <div className="input-with-action">
                    <input
                      type={showKey ? "text" : "password"}
                      value={draft.api_key}
                      onChange={(event) => updateDraft({ api_key: event.target.value })}
                      placeholder={
                        draft.id
                          ? "Để trống để giữ key đã lưu"
                          : keyPlaceholder(draft.provider)
                      }
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="input-action"
                      onClick={() => setShowKey((value) => !value)}
                      aria-label={showKey ? "Ẩn API key" : "Hiện API key"}
                    >
                      {showKey ? <IconEyeOff size={15} /> : <IconEye size={15} />}
                    </button>
                  </div>
                </label>
              </div>

              <p className="field-note">
                Key được mã hóa AES-256-GCM và chỉ lưu trên máy. Với endpoint local như Ollama, có
                thể để trống.
              </p>

              <div className="form-foot">
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={draft.set_active}
                    onChange={(event) => updateDraft({ set_active: event.target.checked })}
                  />
                  <span>Đặt làm nhà cung cấp đang dùng</span>
                </label>
                <div className="form-actions">
                  <button className="btn" onClick={() => setDraft(null)}>
                    Hủy
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => void saveProvider()}
                    disabled={saving}
                  >
                    {saving ? "Đang lưu…" : "Lưu nhà cung cấp"}
                  </button>
                </div>
              </div>
            </div>
          )}
        </section>
      );
    }

    if (tab === "skills") {
      return (
        <section className="settings-section">
          <div className="section-head">
            <div>
              <h3>Kỹ năng</h3>
              <p>
                Prompt pack Markdown được chèn vào context khi khớp môn học và từ khóa trong câu
                hỏi.
              </p>
            </div>
            <button className="btn" onClick={() => void openPath("skills")}>
              <IconFolder size={15} />
              Mở thư mục
            </button>
          </div>

          {extensions && extensions.skills.length === 0 ? (
            <EmptyNote>Chưa có kỹ năng nào.</EmptyNote>
          ) : (
            <div className="skill-grid">
              {extensions?.skills.map((skill) => {
                const key = `${skill.source}:${skill.id}`;
                const expanded = expandedSkill === key;
                const builtin = skill.source === "user" && BUILTIN_SKILL_IDS.has(skill.id);
                return (
                  <article className="skill-card" key={key}>
                    <div className="card-head">
                      <h4>{skill.name}</h4>
                      <Tag
                        tone={builtin ? "muted" : skill.source === "user" ? "forest" : "clay"}
                      >
                        {builtin
                          ? "Có sẵn"
                          : skill.source === "user"
                            ? "Của bạn"
                            : `Plugin · ${skill.source}`}
                      </Tag>
                    </div>
                    {skill.description && <p className="card-copy">{skill.description}</p>}
                    <div className="tag-row">
                      {(skill.subjects.length > 0 ? skill.subjects : ["all"]).map((subject) => (
                        <Tag key={subject} tone="muted">
                          {subject === "all" ? "mọi môn" : subjectLabel(subject)}
                        </Tag>
                      ))}
                    </div>
                    {skill.triggers.length > 0 && (
                      <div className="trigger-line">
                        <span>Từ khóa</span>
                        <code>{skill.triggers.join(" · ")}</code>
                      </div>
                    )}
                    <button
                      className="link-button"
                      onClick={() => setExpandedSkill(expanded ? null : key)}
                    >
                      {expanded ? "Ẩn nội dung" : "Xem nội dung"}
                      <IconChevronDown
                        size={14}
                        className={expanded ? "rotated" : undefined}
                      />
                    </button>
                    {expanded && <pre className="skill-body">{skill.body}</pre>}
                  </article>
                );
              })}
            </div>
          )}

          <Callout>
            Thêm kỹ năng bằng cách tạo <code>skills/&lt;id&gt;/SKILL.md</code> với frontmatter{" "}
            <code>name</code>, <code>subjects</code>, <code>triggers</code>. Không cần build lại.
          </Callout>
        </section>
      );
    }

    if (tab === "mcp") {
      return (
        <section className="settings-section">
          <div className="section-head">
            <div>
              <h3>MCP server</h3>
              <p>
                Công cụ ngoài qua stdio JSON-RPC. Server được khởi động mỗi lượt chat và tắt khi
                xong.
              </p>
            </div>
            {!mcpDraft && (
              <button className="btn btn-primary" onClick={startAddMcp}>
                <IconPlus size={15} />
                Thêm server
              </button>
            )}
          </div>

          {extensions && extensions.mcp.length === 0 ? (
            <EmptyNote>
              Chưa có MCP server. Thêm một server hoặc cài plugin mẫu ở tab Plugin.
            </EmptyNote>
          ) : (
            <div className="server-list">
              {extensions?.mcp.map((server) => {
                const result = mcpTests[server.name];
                return (
                  <div className="server-row" key={server.name}>
                    <div className="server-main">
                      <div className="server-title">
                        <code className="server-name">{server.name}</code>
                        <Tag tone={server.source === "user" ? "forest" : "clay"}>
                          {server.source === "user" ? "Tự thêm" : `Plugin · ${server.source}`}
                        </Tag>
                        {!server.enabled && <Tag tone="neutral">Đang tắt</Tag>}
                      </div>
                      <code className="server-command">
                        {server.command} {server.args.join(" ")}
                      </code>
                      {result === "loading" && (
                        <div className="test-result">Đang khởi động server…</div>
                      )}
                      {result && result !== "loading" && (
                        <div className={`test-result ${result.ok ? "ok" : "fail"}`}>
                          {result.ok ? (
                            <>
                              <span>Kết nối OK · {result.tools.length} công cụ</span>
                              {result.tools.map((tool) => (
                                <code key={tool}>{tool}</code>
                              ))}
                            </>
                          ) : (
                            <span>{result.error ?? "Không rõ lỗi"}</span>
                          )}
                        </div>
                      )}
                    </div>
                    <div className="row-actions">
                      <button
                        className="btn btn-sm"
                        onClick={() => void testMcp(server.name)}
                        disabled={result === "loading"}
                      >
                        {result === "loading" ? "Đang thử…" : "Kiểm tra"}
                      </button>
                      {server.source === "user" && (
                        <>
                          <button
                            className="btn btn-sm btn-ghost"
                            onClick={() => startEditMcp(server)}
                          >
                            <IconPen size={14} />
                            Sửa
                          </button>
                          {confirmMcpDelete === server.name ? (
                            <>
                              <button
                                className="btn btn-sm btn-danger"
                                onClick={() => void removeMcp(server)}
                              >
                                Xóa
                              </button>
                              <button
                                className="btn btn-sm btn-ghost"
                                onClick={() => setConfirmMcpDelete(null)}
                              >
                                Hủy
                              </button>
                            </>
                          ) : (
                            <button
                              className="icon-button icon-danger"
                              aria-label={`Xóa MCP server ${server.name}`}
                              onClick={() => setConfirmMcpDelete(server.name)}
                            >
                              <IconTrash size={15} />
                            </button>
                          )}
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          {mcpDraft && (
            <div className="form-panel">
              <div className="form-title">
                <h4>{mcpDraft.originalName ? "Sửa MCP server" : "Thêm MCP server"}</h4>
                <button
                  className="icon-button"
                  onClick={() => setMcpDraft(null)}
                  aria-label="Đóng form"
                >
                  <IconX size={16} />
                </button>
              </div>

              <div className="form-grid">
                <label className="field">
                  <span>Tên</span>
                  <input
                    value={mcpDraft.name}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, name: event.target.value } : previous,
                      )
                    }
                    placeholder="filesystem"
                    disabled={Boolean(mcpDraft.originalName)}
                    spellCheck={false}
                  />
                </label>
                <label className="field">
                  <span>Command</span>
                  <input
                    value={mcpDraft.command}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, command: event.target.value } : previous,
                      )
                    }
                    placeholder="python3 / npx / uvx"
                    spellCheck={false}
                  />
                </label>
                <label className="field field-wide">
                  <span>Arguments</span>
                  <input
                    value={mcpDraft.argsText}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, argsText: event.target.value } : previous,
                      )
                    }
                    placeholder="-y @modelcontextprotocol/server-filesystem /home/ban"
                    spellCheck={false}
                  />
                </label>
                <label className="field field-wide">
                  <span>Working directory (tùy chọn)</span>
                  <input
                    value={mcpDraft.cwd}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, cwd: event.target.value } : previous,
                      )
                    }
                    placeholder="/home/ban/projects"
                    spellCheck={false}
                  />
                </label>
                <label className="field field-wide">
                  <span>Env (mỗi dòng KEY=VALUE)</span>
                  <textarea
                    rows={3}
                    value={mcpDraft.envText}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, envText: event.target.value } : previous,
                      )
                    }
                    placeholder="API_TOKEN=…"
                    spellCheck={false}
                  />
                </label>
              </div>

              <div className="form-foot">
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={mcpDraft.enabled}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, enabled: event.target.checked } : previous,
                      )
                    }
                  />
                  <span>Bật server này</span>
                </label>
                <div className="form-actions">
                  <button className="btn" onClick={() => setMcpDraft(null)}>
                    Hủy
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => void saveMcp()}
                    disabled={mcpSaving}
                  >
                    {mcpSaving ? "Đang lưu…" : "Lưu server"}
                  </button>
                </div>
              </div>
            </div>
          )}
        </section>
      );
    }

    if (tab === "plugins") {
      return (
        <section className="settings-section">
          <div className="section-head">
            <div>
              <h3>Plugin</h3>
              <p>
                Gói cấu hình gồm kỹ năng và MCP server. Plugin không chạy code tùy ý ngoài những
                server bạn khai báo.
              </p>
            </div>
            <div className="head-actions">
              <button className="btn" onClick={() => void openPath("plugins")}>
                <IconFolder size={15} />
                Mở thư mục
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void installSample()}
                disabled={installing}
              >
                {installing ? "Đang cài…" : "Cài plugin mẫu"}
              </button>
            </div>
          </div>

          {extensions && extensions.plugins.length === 0 ? (
            <EmptyNote>
              Chưa có plugin. Cài plugin mẫu để thử một kỹ năng và MCP server demo.
            </EmptyNote>
          ) : (
            <div className="plugin-list">
              {extensions?.plugins.map((plugin) => (
                <article className="plugin-card" key={plugin.id}>
                  <div className="card-head">
                    <h4>{plugin.name}</h4>
                    <Tag tone="muted">v{plugin.version || "0.1.0"}</Tag>
                  </div>
                  <p className="card-copy">{plugin.description || "Không có mô tả."}</p>
                  <div className="tag-row">
                    <Tag tone="forest">{plugin.skills} kỹ năng</Tag>
                    <Tag tone="clay">{plugin.mcp_servers.length} MCP</Tag>
                    {plugin.mcp_servers.map((name) => (
                      <Tag key={name} tone="muted">
                        {name}
                      </Tag>
                    ))}
                  </div>
                  <code className="path-line">{plugin.path}</code>
                </article>
              ))}
            </div>
          )}

          <Callout>
            Tự viết plugin: tạo <code>plugins/&lt;id&gt;/plugin.json</code> với <code>skills</code>{" "}
            và <code>mcpServers</code>. Thư mục plugin là working directory mặc định cho server.
          </Callout>
        </section>
      );
    }

    return (
      <section className="settings-section">
        <div className="section-head">
          <div>
            <h3>Dữ liệu</h3>
            <p>Mọi thứ nằm trên máy bạn. Không có server trung gian nào giữ lịch sử học.</p>
          </div>
        </div>

        <div className="data-panel">
          <div className="kv">
            <span>Thư mục dữ liệu</span>
            <code>{status?.data_dir ?? extensions?.paths.data_dir ?? "…"}</code>
          </div>
          <div className="kv">
            <span>Phiên bản</span>
            <span>{status?.version ?? "…"}</span>
          </div>
          <div className="kv">
            <span>Buổi học</span>
            <span>{status?.sessions ?? 0}</span>
          </div>
          <div className="kv">
            <span>Kỹ năng</span>
            <span>{status?.extensions.skills ?? 0}</span>
          </div>
          <div className="kv">
            <span>MCP server</span>
            <span>{status?.extensions.mcp_servers ?? 0}</span>
          </div>
          <div className="kv">
            <span>Plugin</span>
            <span>{status?.extensions.plugins ?? 0}</span>
          </div>
        </div>

        <div className="button-row">
          <button className="btn" onClick={() => void openPath("data")}>
            <IconFolder size={15} />
            Mở thư mục dữ liệu
          </button>
          <button className="btn" onClick={() => void openPath("skills")}>
            <IconBook size={15} />
            Mở skills
          </button>
          <button className="btn" onClick={() => void openPath("plugins")}>
            <IconPuzzle size={15} />
            Mở plugins
          </button>
          <button className="btn" onClick={() => void openPath("mcp")}>
            <IconTerminal size={15} />
            Mở mcp.json
          </button>
        </div>

        <Callout>
          <strong>Quyền riêng tư.</strong> API key được mã hóa AES-256-GCM, khóa nằm trong{" "}
          <code>master.key</code>. Lịch sử chat lưu trong <code>tez.db</code> (SQLite). Xóa thư mục
          dữ liệu là xóa toàn bộ.
        </Callout>
      </section>
    );
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="settings-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Cài đặt"
        ref={panelRef}
        tabIndex={-1}
      >
        <nav className="settings-nav" aria-label="Mục cài đặt">
          <div className="settings-nav-head">
            <div className="settings-nav-brand">
              <span className="brand-mark settings-brand-mark" aria-hidden="true">
                t
              </span>
              <span className="settings-nav-title">tez</span>
            </div>
            <p>Cài đặt bàn học</p>
          </div>
          <div className="settings-tabs">
            {TAB_META.map((item) => (
              <button
                key={item.id}
                className={`settings-tab ${tab === item.id ? "is-active" : ""}`}
                onClick={() => onTab(item.id)}
                aria-current={tab === item.id ? "page" : undefined}
              >
                <span className="settings-tab-icon">{item.icon}</span>
                <span>{item.label}</span>
                {counts[item.id] !== null && (
                  <span className="settings-tab-count">{counts[item.id]}</span>
                )}
              </button>
            ))}
          </div>
          <div className="settings-nav-foot">
            <span className="status-dot is-on" aria-hidden="true" />
            <span>Dữ liệu local</span>
          </div>
        </nav>

        <div className="settings-main">
          <header className="settings-header">
            <div>
              <h2>Cài đặt</h2>
              <p>Nhà cung cấp, kỹ năng và công cụ của bàn học.</p>
            </div>
            <button className="icon-button" onClick={onClose} aria-label="Đóng cài đặt">
              <IconX size={18} />
            </button>
          </header>
          <div className="settings-content">{content()}</div>
        </div>
      </div>
    </div>
  );
}
