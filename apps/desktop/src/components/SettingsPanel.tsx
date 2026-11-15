import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import * as api from "../api";
import type {
  AppStatus,
  BackupInfo,
  ExtensionKind,
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
  IconSliders,
  IconSparkle,
  IconTerminal,
  IconTrash,
  IconX,
} from "../icons";
import { Callout, EmptyNote, Spinner, Tag } from "./ui";
import { CreateWithAI } from "./CreateWithAI";
import { HarnessPanel } from "./HarnessPanel";
import { McpExtrasDialog } from "./McpExtrasDialog";
import { UpdatePanel } from "./UpdatePanel";

type Props = {
  open: boolean;
  tab: SettingsTab;
  status: AppStatus | null;
  onTab: (tab: SettingsTab) => void;
  onClose: () => void;
  onChanged: () => void | Promise<void>;
  notify: (kind: ToastKind, title: string, detail?: string) => void;
  createRequest?: ExtensionKind | null;
  onCreateRequest?: () => void;
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
  read_only: boolean;
  url: string;
  headersText: string;
  bearerTokenEnv: string;
  oauthAuthorizationUrl: string;
  oauthTokenUrl: string;
  oauthClientId: string;
  oauthClientSecretEnv: string;
  oauthScopesText: string;
  oauthRedirectUri: string;
  oauthResource: string;
  sandboxEnabled: boolean;
  sandboxAllowNetwork: boolean;
  sandboxWritableText: string;
};

const PRESETS: Preset[] = [
  {
    id: "groq",
    name: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    model: "openai/gpt-oss-120b",
    keyHint: "gsk_…",
    note: "Free tier, phản hồi rất nhanh",
  },
  {
    id: "gemini",
    name: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    model: "gemini-3.5-flash",
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
  { id: "providers", label: "Model", icon: <IconKey size={16} /> },
  { id: "harness", label: "Agent", icon: <IconSliders size={16} /> },
  { id: "skills", label: "Kỹ năng", icon: <IconBook size={16} /> },
  { id: "mcp", label: "MCP", icon: <IconPlug size={16} /> },
  { id: "plugins", label: "Plugin", icon: <IconPuzzle size={16} /> },
  { id: "data", label: "Dữ liệu", icon: <IconDatabase size={16} /> },
  { id: "update", label: "Cập nhật", icon: <IconDownload size={16} /> },
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

export function SettingsPanel({
  open,
  tab,
  status,
  onTab,
  onClose,
  onChanged,
  notify,
  createRequest,
  onCreateRequest,
}: Props) {
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
  const [oauthBusy, setOauthBusy] = useState<string | null>(null);
  const [extrasServer, setExtrasServer] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [createKind, setCreateKind] = useState<ExtensionKind | null>(null);
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [backupBusy, setBackupBusy] = useState(false);
  const [restorePath, setRestorePath] = useState("");
  const panelRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [nextProviders, nextExtensions, nextBackups] = await Promise.all([
        api.listProviders(),
        api.listExtensions(),
        api.listBackups(),
      ]);
      setProviders(nextProviders);
      setExtensions(nextExtensions);
      setBackups(nextBackups);
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
    if (!open || !createRequest) return;
    setCreateKind(createRequest);
    onCreateRequest?.();
  }, [open, createRequest, onCreateRequest]);

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
    harness: null,
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
        api_key: draft.id && draft.api_key === "" ? null : draft.api_key.trim(),
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
      read_only: false,
      url: "",
      headersText: "",
      bearerTokenEnv: "",
      oauthAuthorizationUrl: "",
      oauthTokenUrl: "",
      oauthClientId: "",
      oauthClientSecretEnv: "",
      oauthScopesText: "",
      oauthRedirectUri: "",
      oauthResource: "",
      sandboxEnabled: false,
      sandboxAllowNetwork: false,
      sandboxWritableText: "",
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
      read_only: server.read_only,
      url: server.url ?? "",
      headersText: Object.entries(server.headers)
        .map(([key, value]) => `${key}=${value}`)
        .join("\n"),
      bearerTokenEnv: server.bearer_token_env ?? "",
      oauthAuthorizationUrl: server.oauth?.authorization_url ?? "",
      oauthTokenUrl: server.oauth?.token_url ?? "",
      oauthClientId: server.oauth?.client_id ?? "",
      oauthClientSecretEnv: server.oauth?.client_secret_env ?? "",
      oauthScopesText: server.oauth?.scopes.join(" ") ?? "",
      oauthRedirectUri: server.oauth?.redirect_uri ?? "",
      oauthResource: server.oauth?.resource ?? "",
      sandboxEnabled: server.sandbox?.enabled ?? false,
      sandboxAllowNetwork: server.sandbox?.allow_network ?? false,
      sandboxWritableText: (server.sandbox?.writable_paths ?? []).join("\n"),
    });
  };

  const saveMcp = async () => {
    if (!mcpDraft) return;
    const name = (mcpDraft.originalName ?? mcpDraft.name).trim();
    const command = mcpDraft.command.trim();
    const url = mcpDraft.url.trim();
    if (!name) {
      notify("error", "Thiếu tên MCP server");
      return;
    }
    if (!command && !url) {
      notify("error", "Thiếu command hoặc URL", "stdio dùng command; HTTP dùng URL.");
      return;
    }

    let env: Record<string, string>;
    let headers: Record<string, string>;
    try {
      env = parseEnv(mcpDraft.envText);
      headers = parseEnv(mcpDraft.headersText);
    } catch (error) {
      notify("error", "Env/header không hợp lệ", api.errorMessage(error));
      return;
    }

    const oauth =
      mcpDraft.oauthAuthorizationUrl.trim() ||
      mcpDraft.oauthTokenUrl.trim() ||
      mcpDraft.oauthClientId.trim()
        ? {
            authorization_url: mcpDraft.oauthAuthorizationUrl.trim(),
            token_url: mcpDraft.oauthTokenUrl.trim(),
            client_id: mcpDraft.oauthClientId.trim(),
            client_secret_env: mcpDraft.oauthClientSecretEnv.trim() || null,
            client_secret: null,
            scopes: mcpDraft.oauthScopesText
              .split(/[\s,]+/)
              .map((value) => value.trim())
              .filter(Boolean),
            redirect_uri: mcpDraft.oauthRedirectUri.trim() || null,
            resource: mcpDraft.oauthResource.trim() || null,
          }
        : null;

    setMcpSaving(true);
    try {
      await api.saveMcpServer({
        name,
        command,
        args: splitArgs(mcpDraft.argsText),
        env,
        cwd: mcpDraft.cwd.trim() || null,
        enabled: mcpDraft.enabled,
        read_only: mcpDraft.read_only,
        url: url || null,
        headers,
        bearer_token_env: mcpDraft.bearerTokenEnv.trim() || null,
        oauth,
        sandbox: {
          enabled: mcpDraft.sandboxEnabled,
          allow_network: mcpDraft.sandboxAllowNetwork,
          writable_paths: mcpDraft.sandboxWritableText
            .split("\n")
            .map((value) => value.trim())
            .filter(Boolean),
        },
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

  const connectOAuth = async (name: string) => {
    setOauthBusy(name);
    try {
      await api.startMcpOAuth(name);
      await reload();
      await onChanged();
      notify("success", "Đã kết nối OAuth", name);
    } catch (error) {
      notify("error", "Không kết nối được OAuth", api.errorMessage(error));
    } finally {
      setOauthBusy(null);
    }
  };

  const disconnectOAuth = async (name: string) => {
    setOauthBusy(name);
    try {
      await api.disconnectMcpOAuth(name);
      await reload();
      await onChanged();
      notify("success", "Đã ngắt OAuth", name);
    } catch (error) {
      notify("error", "Không ngắt được OAuth", api.errorMessage(error));
    } finally {
      setOauthBusy(null);
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

  const makeBackup = async () => {
    setBackupBusy(true);
    try {
      const result = await api.createBackup(null);
      setBackups(await api.listBackups());
      notify(
        "success",
        "Đã sao lưu dữ liệu",
        `${result.files} file · ${result.path}`,
      );
    } catch (error) {
      notify("error", "Không sao lưu được", api.errorMessage(error));
    } finally {
      setBackupBusy(false);
    }
  };

  const restoreFromBackup = async () => {
    const path = restorePath.trim();
    if (!path) {
      notify("info", "Chưa nhập đường dẫn", "Dán đường dẫn thư mục bản sao lưu có manifest.json.");
      return;
    }
    if (
      !window.confirm(
        "Phục hồi dữ liệu từ bản sao lưu này? Dữ liệu hiện tại sẽ được sao lưu an toàn trước.",
      )
    ) {
      return;
    }
    setBackupBusy(true);
    try {
      const result = await api.restoreBackup(path, true);
      notify(
        "success",
        "Đã phục hồi dữ liệu",
        `${result.files} file${result.safety_backup ? ` · bản an toàn: ${result.safety_backup}` : ""}`,
      );
      setRestorePath("");
      await reload();
      await onChanged();
    } catch (error) {
      notify("error", "Không phục hồi được", api.errorMessage(error));
    } finally {
      setBackupBusy(false);
    }
  };

  const content = () => {
    if (tab === "harness") {
      return <HarnessPanel notify={notify} onChanged={onChanged} />;
    }

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
                    onChange={(event) => {
                      const preset = PRESETS.find((item) => item.id === event.target.value);
                      if (preset) applyPreset(preset);
                      else updateDraft({ provider: event.target.value });
                    }}
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
                Prompt pack Markdown được chèn vào context khi khớp ngữ cảnh và từ khóa trong câu
                hỏi. Có thể tạo bằng AI rồi chỉnh lại file. Skill riêng của dự án nằm trong
                <code> &lt;workspace&gt;/.tez/skills/</code> và được nạp khi chọn workspace.
              </p>
            </div>
            <div className="head-actions">
              <button className="btn" onClick={() => void openPath("skills")}>
                <IconFolder size={15} />
                Mở thư mục
              </button>
              <button className="btn btn-primary" onClick={() => setCreateKind("skill")}>
                <IconSparkle size={15} />
                Tạo với AI
              </button>
            </div>
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
                          {subject === "all" ? "mọi ngữ cảnh" : subjectLabel(subject)}
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
              <div className="head-actions">
                <button className="btn btn-primary" onClick={startAddMcp}>
                  <IconPlus size={15} />
                  Thêm server
                </button>
                <button className="btn" onClick={() => setCreateKind("mcp")}>
                  <IconSparkle size={15} />
                  Tạo với AI
                </button>
              </div>
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
                        {server.url && <Tag tone="neutral">HTTP</Tag>}
                        {!server.enabled && <Tag tone="neutral">Đang tắt</Tag>}
                        {server.read_only && <Tag tone="forest">Chỉ đọc</Tag>}
                        {server.sandbox?.enabled && <Tag tone="forest">Sandbox</Tag>}
                        {server.oauth && <Tag tone="clay">OAuth</Tag>}
                        {server.oauth &&
                          (server.authorized ? (
                            <Tag tone="forest">Đã kết nối</Tag>
                          ) : (
                            <Tag tone="neutral">Cần kết nối</Tag>
                          ))}
                      </div>
                      <code className="server-command">
                        {server.url ?? `${server.command} ${server.args.join(" ")}`}
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
                      {server.oauth &&
                        (server.authorized ? (
                          <button
                            className="btn btn-sm btn-ghost"
                            onClick={() => void disconnectOAuth(server.name)}
                            disabled={oauthBusy === server.name}
                          >
                            {oauthBusy === server.name ? "Đang ngắt…" : "Ngắt OAuth"}
                          </button>
                        ) : (
                          <button
                            className="btn btn-sm btn-primary"
                            onClick={() => void connectOAuth(server.name)}
                            disabled={oauthBusy === server.name || !server.enabled}
                          >
                            {oauthBusy === server.name ? "Đang chờ…" : "Kết nối OAuth"}
                          </button>
                        ))}
                      <button
                        className="btn btn-sm btn-ghost"
                        onClick={() => setExtrasServer(server.name)}
                        disabled={!server.enabled}
                      >
                        Tài nguyên
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
                  <span>Command (stdio)</span>
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
                  <span>URL (Streamable HTTP, tùy chọn)</span>
                  <input
                    value={mcpDraft.url}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, url: event.target.value } : previous,
                      )
                    }
                    placeholder="https://mcp.example.com/mcp"
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
                  <span>Sandbox: thư mục ghi thêm (mỗi dòng một đường dẫn)</span>
                  <textarea
                    rows={2}
                    value={mcpDraft.sandboxWritableText}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous
                          ? { ...previous, sandboxWritableText: event.target.value }
                          : previous,
                      )
                    }
                    placeholder="~/.npm"
                    spellCheck={false}
                    disabled={!mcpDraft.sandboxEnabled}
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
                <label className="field field-wide">
                  <span>HTTP headers (mỗi dòng KEY=VALUE)</span>
                  <textarea
                    rows={2}
                    value={mcpDraft.headersText}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, headersText: event.target.value } : previous,
                      )
                    }
                    placeholder="Authorization=Bearer …"
                    spellCheck={false}
                  />
                </label>
                <label className="field field-wide">
                  <span>Bearer token env (tùy chọn)</span>
                  <input
                    value={mcpDraft.bearerTokenEnv}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous
                          ? { ...previous, bearerTokenEnv: event.target.value }
                          : previous,
                      )
                    }
                    placeholder="TEZ_MCP_TOKEN"
                    spellCheck={false}
                  />
                  <p className="field-note">
                    Đọc từ biến môi trường lúc kết nối; token không được lưu vào mcp.json.
                  </p>
                </label>
                <details className="oauth-fields field-wide">
                  <summary>OAuth 2.1 + PKCE (tùy chọn)</summary>
                  <p className="field-note">
                    Cần đăng ký redirect URI với provider. Token lưu trong oauth.json (0600),
                    không nằm trong mcp.json.
                  </p>
                  <div className="field-grid">
                    <label className="field field-wide">
                      <span>Authorization URL</span>
                      <input
                        value={mcpDraft.oauthAuthorizationUrl}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous
                              ? { ...previous, oauthAuthorizationUrl: event.target.value }
                              : previous,
                          )
                        }
                        placeholder="https://auth.example.com/authorize"
                        spellCheck={false}
                      />
                    </label>
                    <label className="field field-wide">
                      <span>Token URL</span>
                      <input
                        value={mcpDraft.oauthTokenUrl}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous ? { ...previous, oauthTokenUrl: event.target.value } : previous,
                          )
                        }
                        placeholder="https://auth.example.com/token"
                        spellCheck={false}
                      />
                    </label>
                    <label className="field">
                      <span>Client ID</span>
                      <input
                        value={mcpDraft.oauthClientId}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous ? { ...previous, oauthClientId: event.target.value } : previous,
                          )
                        }
                        spellCheck={false}
                      />
                    </label>
                    <label className="field">
                      <span>Client secret env</span>
                      <input
                        value={mcpDraft.oauthClientSecretEnv}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous
                              ? { ...previous, oauthClientSecretEnv: event.target.value }
                              : previous,
                          )
                        }
                        placeholder="TEZ_MCP_CLIENT_SECRET"
                        spellCheck={false}
                      />
                    </label>
                    <label className="field field-wide">
                      <span>Scopes</span>
                      <input
                        value={mcpDraft.oauthScopesText}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous
                              ? { ...previous, oauthScopesText: event.target.value }
                              : previous,
                          )
                        }
                        placeholder="read write"
                        spellCheck={false}
                      />
                    </label>
                    <label className="field">
                      <span>Redirect URI</span>
                      <input
                        value={mcpDraft.oauthRedirectUri}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous
                              ? { ...previous, oauthRedirectUri: event.target.value }
                              : previous,
                          )
                        }
                        placeholder="http://127.0.0.1:33418/callback"
                        spellCheck={false}
                      />
                    </label>
                    <label className="field">
                      <span>Resource</span>
                      <input
                        value={mcpDraft.oauthResource}
                        onChange={(event) =>
                          setMcpDraft((previous) =>
                            previous ? { ...previous, oauthResource: event.target.value } : previous,
                          )
                        }
                        placeholder="https://mcp.example.com"
                        spellCheck={false}
                      />
                    </label>
                  </div>
                </details>
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
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={mcpDraft.read_only}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous ? { ...previous, read_only: event.target.checked } : previous,
                      )
                    }
                  />
                  <span>Chỉ đọc (cho phép trong Plan)</span>
                </label>
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={mcpDraft.sandboxEnabled}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous
                          ? { ...previous, sandboxEnabled: event.target.checked }
                          : previous,
                      )
                    }
                  />
                  <span>Sandbox stdio (Linux, bubblewrap)</span>
                </label>
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={mcpDraft.sandboxAllowNetwork}
                    onChange={(event) =>
                      setMcpDraft((previous) =>
                        previous
                          ? { ...previous, sandboxAllowNetwork: event.target.checked }
                          : previous,
                      )
                    }
                    disabled={!mcpDraft.sandboxEnabled}
                  />
                  <span>Cho phép mạng trong sandbox</span>
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
              <button className="btn" onClick={() => void installSample()} disabled={installing}>
                {installing ? "Đang cài…" : "Cài plugin mẫu"}
              </button>
              <button className="btn btn-primary" onClick={() => setCreateKind("plugin")}>
                <IconSparkle size={15} />
                Tạo với AI
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
            <p>Mọi thứ nằm trên máy bạn. Không có server trung gian nào giữ lịch sử phiên.</p>
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
            <span>Phiên</span>
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

        <div className="data-panel">
          <div className="kv">
            <span>Bản sao lưu</span>
            <span>{backups.length} bản</span>
          </div>
          {backups.slice(0, 3).map((backup) => (
            <div className="kv" key={backup.path}>
              <code>{backup.path}</code>
              <span>
                {backup.files} file · {new Date(backup.created_at_unix * 1000).toLocaleString()}
              </span>
            </div>
          ))}
          {backups.length === 0 && (
            <p className="card-copy">Chưa có bản sao lưu. Tạo bản đầu tiên trước khi thử nghiệm.</p>
          )}
        </div>

        <div className="button-row">
          <button className="btn" disabled={backupBusy} onClick={() => void makeBackup()}>
            <IconDatabase size={15} />
            {backupBusy ? "Đang xử lý…" : "Sao lưu ngay"}
          </button>
        </div>

        <div className="field">
          <label className="field-label" htmlFor="restore-path">
            Phục hồi từ thư mục bản sao lưu
          </label>
          <input
            id="restore-path"
            className="input"
            placeholder="/path/to/tez-backup-…"
            value={restorePath}
            onChange={(event) => setRestorePath(event.target.value)}
          />
          <button
            className="btn"
            disabled={backupBusy}
            onClick={() => void restoreFromBackup()}
          >
            <IconRefresh size={15} />
            Phục hồi
          </button>
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
            <p>Cài đặt agent</p>
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
              <p>Model, hành vi agent, kỹ năng, MCP và plugin.</p>
            </div>
            <button className="icon-button" onClick={onClose} aria-label="Đóng cài đặt">
              <IconX size={18} />
            </button>
          </header>
          <div className="settings-content">{content()}</div>
        </div>
      </div>

      <CreateWithAI
        open={createKind !== null}
        kind={createKind}
        onClose={() => setCreateKind(null)}
        onSaved={async () => {
          await reload();
          await onChanged();
        }}
        notify={notify}
      />

      <McpExtrasDialog
        open={extrasServer !== null}
        serverName={extrasServer}
        onClose={() => setExtrasServer(null)}
        notify={notify}
      />
    </div>
  );
}
