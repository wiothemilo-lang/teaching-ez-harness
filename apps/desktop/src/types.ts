export type SettingsTab =
  | "providers"
  | "skills"
  | "mcp"
  | "plugins"
  | "harness"
  | "update"
  | "data";

export type AppMode = "chat" | "plan" | "code";

export type PermissionMode = "read_only" | "strict" | "standard" | "trust";

export type TodoStatus = "pending" | "in_progress" | "completed";

export type TodoItem = {
  content: string;
  status: TodoStatus;
};

export type CompactReport = {
  persisted: number;
  snipped: number;
  micro_compacted: number;
  summarized: boolean;
  estimated_chars: number;
  transcript: string | null;
};

export type CompactionSettings = {
  enabled: boolean;
  context_char_limit: number;
  large_result_chars: number;
  preview_chars: number;
  trigger_messages: number;
  target_messages: number;
  keep_recent_results: number;
};

export type MemorySettings = {
  enabled: boolean;
  auto_recall: boolean;
  recall_limit: number;
};

export type HookConfig = {
  event: string;
  command: string;
  timeout_secs: number;
  enabled: boolean;
};

export type PermissionRule = {
  action: "allow" | "ask" | "deny";
  tool: string;
  pattern: string;
};

export type HarnessConfig = {
  permission_mode: PermissionMode;
  permission_rules: PermissionRule[];
  max_steps: number;
  max_tokens: number;
  temperature: number;
  top_p: number;
  max_tool_output: number;
  subagent_max_steps: number;
  system_prompt: string;
  language: string;
  disabled_tools: string[];
  compaction: CompactionSettings;
  memory: MemorySettings;
  hooks: HookConfig[];
};

export type MemoryRecord = {
  name: string;
  description: string;
  kind: string;
  body: string;
  path: string;
};

export type HarnessInfo = {
  config: HarnessConfig;
  path: string;
  memory_dir: string;
  audit_log: string;
  memory: MemoryRecord[];
};

export type ApprovalRequest = {
  id: string;
  tool: string;
  args: unknown;
  reason: string;
  session_id: string | null;
  preview?: string | null;
};

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
  todos: TodoItem[];
  compactions: CompactReport;
  permission_denials: number;
  stop_reason: "completed" | "max_steps" | "stopped";
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  text: string;
  images?: string[];
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
  messages: { role: string; content: string; images?: string[] }[];
  todos: TodoItem[];
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
  read_only: boolean;
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

export type PlanInfo = {
  path: string;
  text: string;
};

export type DraftFile = {
  path: string;
  content: string;
};

export type DraftExtensionResult = {
  kind: string;
  files: DraftFile[];
  notes: string;
};

export type ExtensionKind = "skill" | "mcp" | "plugin";

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
  read_only: boolean;
};

export type ChatPayload = {
  message: string;
  images: string[];
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
