import { invoke } from "@tauri-apps/api/core";
import type {
  AppStatus,
  BackupInfo,
  BackupResult,
  ChatPayload,
  ChatReply,
  ConnectionTest,
  ExtensionsInfo,
  HarnessConfig,
  HarnessInfo,
  McpTest,
  OAuthTokenStatus,
  PlanInfo,
  PluginInfo,
  ProviderInfo,
  SaveMcpPayload,
  SaveProviderPayload,
  DraftFile,
  DraftExtensionResult,
  ExtensionKind,
  SessionDetail,
  SessionInfo,
  DownloadUpdateResult,
  UpdateInfo,
} from "./types";

export function appStatus(): Promise<AppStatus> {
  return invoke("app_status");
}

export function listSessions(limit = 20): Promise<SessionInfo[]> {
  return invoke("list_sessions", { limit });
}

export function loadSession(sessionId: string): Promise<SessionDetail> {
  return invoke("load_session", { request: { session_id: sessionId } });
}

export function chat(payload: ChatPayload): Promise<ChatReply> {
  return invoke("chat", { request: payload });
}

export function stopChat(): Promise<boolean> {
  return invoke("stop_chat");
}

export function loadSessionPlan(sessionId: string): Promise<PlanInfo> {
  return invoke("load_session_plan", { sessionId });
}

export function saveSessionPlan(
  sessionId: string,
  text: string,
  allowIncomplete = false,
): Promise<PlanInfo> {
  return invoke("save_session_plan", { sessionId, text, allowIncomplete });
}

export function respondApproval(
  id: string,
  approved: boolean,
  remember = false,
): Promise<boolean> {
  return invoke("respond_approval", { request: { id, approved, remember } });
}

export function getHarness(): Promise<HarnessInfo> {
  return invoke("get_harness");
}

export function saveHarness(config: HarnessConfig): Promise<HarnessInfo> {
  return invoke("save_harness", { config });
}

export function clearApprovals(): Promise<boolean> {
  return invoke("clear_approvals");
}

export function deleteMemory(name: string): Promise<boolean> {
  return invoke("delete_memory", { request: { name } });
}

export function listProviders(): Promise<ProviderInfo[]> {
  return invoke("list_providers");
}

export function saveProvider(payload: SaveProviderPayload): Promise<ProviderInfo> {
  return invoke("save_provider", { request: payload });
}

export function deleteProvider(id: string): Promise<boolean> {
  return invoke("delete_provider", { id });
}

export function activateProvider(id: string): Promise<ProviderInfo> {
  return invoke("activate_provider", { id });
}

export function testProvider(id: string | null): Promise<ConnectionTest> {
  return invoke("test_provider", { id });
}

export function listExtensions(): Promise<ExtensionsInfo> {
  return invoke("list_extensions");
}

export function draftExtension(
  kind: ExtensionKind,
  prompt: string,
): Promise<DraftExtensionResult> {
  return invoke("draft_extension", { request: { kind, prompt } });
}

export function saveExtension(kind: ExtensionKind, files: DraftFile[]): Promise<string[]> {
  return invoke("save_extension", { request: { kind, files } });
}

export function saveMcpServer(payload: SaveMcpPayload): Promise<void> {
  return invoke("save_mcp_server", { request: payload });
}

export function deleteMcpServer(name: string): Promise<boolean> {
  return invoke("delete_mcp_server", { name });
}

export function testMcpServer(name: string): Promise<McpTest> {
  return invoke("test_mcp_server", { name });
}

export function mcpOAuthStatus(name: string): Promise<OAuthTokenStatus> {
  return invoke("mcp_oauth_status", { name });
}

export function startMcpOAuth(name: string): Promise<OAuthTokenStatus> {
  return invoke("start_mcp_oauth", { name });
}

export function disconnectMcpOAuth(name: string): Promise<boolean> {
  return invoke("disconnect_mcp_oauth", { name });
}

export function mcpExtras(
  name: string,
  action: string,
  uri?: string,
  promptName?: string,
  args?: Record<string, unknown>,
): Promise<string> {
  return invoke("mcp_extras", {
    name,
    action,
    uri: uri ?? null,
    promptName: promptName ?? null,
    arguments: args ?? null,
  });
}

export function installSamplePlugin(): Promise<PluginInfo> {
  return invoke("install_sample_plugin");
}

export function openExtensionPath(kind: "skills" | "plugins" | "data" | "mcp"): Promise<string> {
  return invoke("open_extension_path", { kind });
}

export function checkUpdate(): Promise<UpdateInfo> {
  return invoke("check_update");
}

export function downloadUpdate(): Promise<DownloadUpdateResult> {
  return invoke("download_update");
}

export function launchUpdate(path: string): Promise<void> {
  return invoke("launch_update", { path });
}

export function openExternal(url: string): Promise<void> {
  return invoke("open_external", { url });
}

export function openWorkspace(path: string | null): Promise<{ path: string; exists: boolean }> {
  return invoke("open_workspace", { path });
}

export function listBackups(): Promise<BackupInfo[]> {
  return invoke("list_backups");
}

export function createBackup(out: string | null): Promise<BackupResult> {
  return invoke("create_backup", { out });
}

export function restoreBackup(path: string, yes: boolean): Promise<BackupResult> {
  return invoke("restore_backup", { path, yes });
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}
