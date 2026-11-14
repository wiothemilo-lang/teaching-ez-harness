export type SettingsTab = "providers" | "skills" | "mcp" | "plugins" | "update" | "data";

export type AppMode = "chat" | "code";

export type SubjectInfo = {
  id: string;
  name: string;
  strategy: string;
  tools: string[];
};

export type ConnectionInfo = {
  id: string;
  name: string;
  provider: string;
  model: string;
  base_url: string;
};

export type ProviderInfo = {
  id: string;
  name: string;
  provider: string;
  base_url: string;
  model: string;
  has_key: boolean;
  active: boolean;
};

export type ExtensionSummary = {
  skills: number;
  mcp_servers: number;
  plugins: number;
};

export type AppStatus = {
  connection: ConnectionInfo | null;
  providers: ProviderInfo[];
  subjects: SubjectInfo[];
  sessions: number;
  data_dir: string;
  default_workspace: string;
  version: string;
  extensions: ExtensionSummary;
};

export type UpdateAsset = {
  name: string;
  url: string;
  size: number;
};

export type UpdateInfo = {
  current: string;
  latest: string;
  available: boolean;
  notes: string;
  url: string;
  published_at: string | null;
  assets: UpdateAsset[];
};

export type DownloadUpdateResult = {
  path: string;
  version: string;
};

export type ToolTrace = {
  tool: string;
  args: string;
  output: string;
  ok: boolean;
  duration_ms: number;
};

export type ChatReply = {
  session_id: string;
  text: string;
  tools: ToolTrace[];
  input_tokens: number;
  output_tokens: number;
  provider: string;
  model: string;
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  text: string;
  tools?: ToolTrace[];
};

export type SessionInfo = {
  id: string;
  subject: string | null;
  mode: string;
  state: string;
  messages: number;
  started_at: string;
  summary: string | null;
};

export type SessionDetail = {
  id: string;
  messages: { role: string; content: string }[];
};

export type ConnectionTest = {
  ok: boolean;
  message: string;
  latency_ms: number;
};

export type SkillInfo = {
  id: string;
  name: string;
  description: string;
  subjects: string[];
  triggers: string[];
  body: string;
  source: string;
};

export type McpServerInfo = {
  name: string;
  source: string;
  command: string;
  args: string[];
  cwd: string | null;
  env: Record<string, string>;
  enabled: boolean;
};

export type PluginInfo = {
  id: string;
  name: string;
  version: string;
  description: string;
  path: string;
  skills: number;
  mcp_servers: string[];
};

export type ExtensionPaths = {
  data_dir: string;
  skills_dir: string;
  plugins_dir: string;
  mcp_config: string;
};

export type ExtensionsInfo = {
  skills: SkillInfo[];
  mcp: McpServerInfo[];
  plugins: PluginInfo[];
  paths: ExtensionPaths;
};

export type McpTest = {
  ok: boolean;
  tools: string[];
  error: string | null;
};

export type SaveProviderPayload = {
  id?: string | null;
  name: string;
  provider: string;
  base_url: string | null;
  model: string | null;
  api_key: string | null;
  set_active: boolean;
};

export type SaveMcpPayload = {
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  enabled: boolean;
};

export type ChatPayload = {
  subject: string;
  grade: string;
  message: string;
  session_id: string | null;
  provider_id: string | null;
  mode: AppMode;
  workspace: string | null;
};

export type ToastKind = "success" | "error" | "info";

export type ToastItem = {
  id: string;
  kind: ToastKind;
  title: string;
  detail?: string;
};
