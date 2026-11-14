import { invoke } from "@tauri-apps/api/core";
import type {
  AppStatus,
  ChatPayload,
  ChatReply,
  ConnectionTest,
  ExtensionsInfo,
  McpTest,
  PluginInfo,
  ProviderInfo,
  SaveMcpPayload,
  SaveProviderPayload,
  SessionDetail,
  SessionInfo,
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

export function saveMcpServer(payload: SaveMcpPayload): Promise<void> {
  return invoke("save_mcp_server", { request: payload });
}

export function deleteMcpServer(name: string): Promise<boolean> {
  return invoke("delete_mcp_server", { name });
}

export function testMcpServer(name: string): Promise<McpTest> {
  return invoke("test_mcp_server", { name });
}

export function installSamplePlugin(): Promise<PluginInfo> {
  return invoke("install_sample_plugin");
}

export function openExtensionPath(kind: "skills" | "plugins" | "data" | "mcp"): Promise<string> {
  return invoke("open_extension_path", { kind });
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
