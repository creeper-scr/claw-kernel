/**
 * claw-kernel TypeScript SDK — Protocol Types
 *
 * Mirrors the Rust types in crates/claw-server/src/protocol.rs.
 * All fields use camelCase-compatible snake_case to match JSON serialisation.
 */

// ─── JSON-RPC 2.0 core ────────────────────────────────────────────────────────

export type RequestId = string | number | null;

export interface RpcRequest {
  jsonrpc: '2.0';
  method: string;
  params?: unknown;
  id: RequestId;
}

export interface RpcResponse {
  jsonrpc: '2.0';
  result?: unknown;
  error?: RpcError;
  id: RequestId;
}

export interface RpcNotification {
  jsonrpc: '2.0';
  method: string;
  params?: unknown;
}

export interface RpcError {
  code: number;
  message: string;
  data?: unknown;
}

// ─── Error codes (mirrors error_codes module) ─────────────────────────────────

export const ErrorCode = {
  PARSE_ERROR: -32700,
  INVALID_REQUEST: -32600,
  METHOD_NOT_FOUND: -32601,
  INVALID_PARAMS: -32602,
  INTERNAL_ERROR: -32603,
  SESSION_NOT_FOUND: -32000,
  MAX_SESSIONS_REACHED: -32001,
  PROVIDER_ERROR: -32002,
  AGENT_ERROR: -32003,
  DAEMON_ALREADY_RUNNING: -32004,
  PROVIDER_NOT_FOUND: -32005,
} as const;

// ─── Session API ──────────────────────────────────────────────────────────────

export interface ExternalToolDef {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

export interface SessionConfig {
  system_prompt?: string;
  max_turns?: number;
  provider_override?: string;
  model_override?: string;
  tools?: ExternalToolDef[];
  persist_history?: boolean;
}

export interface CreateSessionParams {
  config?: SessionConfig;
}

export interface CreateSessionResult {
  session_id: string;
}

export interface SendMessageParams {
  session_id: string;
  content: string;
  metadata?: Record<string, unknown>;
}

export interface ToolResultParams {
  session_id: string;
  tool_call_id: string;
  result: unknown;
  success: boolean;
}

export interface DestroySessionParams {
  session_id: string;
}

// ─── Notification params ──────────────────────────────────────────────────────

export interface ChunkParams {
  session_id: string;
  delta: string;
  done: boolean;
}

export interface ToolCallParams {
  session_id: string;
  tool_call_id: string;
  tool_name: string;
  arguments: Record<string, unknown>;
}

export interface FinishParams {
  session_id: string;
  content?: string;
  reason: string;
}

// ─── kernel.info result ───────────────────────────────────────────────────────

export interface KernelInfoResult {
  version: string;
  protocol_version: number;
  providers: string[];
  active_provider: string;
  active_model: string;
  features: string[];
  max_sessions: number;
  current_sessions: number;
}

// ─── Schedule API ─────────────────────────────────────────────────────────────

export interface ScheduleCreateParams {
  session_id: string;
  cron: string;
  prompt: string;
  label?: string;
}

export interface ScheduleCancelParams {
  task_id: string;
}

export interface ScheduleListParams {
  session_id: string;
}

export interface ScheduledTaskInfo {
  task_id: string;
  cron: string;
  label?: string;
  status: string;
}

// ─── Channel API ──────────────────────────────────────────────────────────────

export interface ChannelRegisterParams {
  type: string;
  channel_id: string;
  config: Record<string, unknown>;
}

export interface ChannelUnregisterParams {
  channel_id: string;
}

export interface ChannelInboundParams {
  channel_id: string;
  sender_id: string;
  content: string;
  thread_id?: string;
  message_id?: string;
  metadata?: Record<string, unknown>;
}

export interface ChannelRouteAddParams {
  rule_type: string;
  channel_id?: string;
  sender_id?: string;
  pattern?: string;
  agent_id: string;
}

export interface ChannelRouteRemoveParams {
  agent_id: string;
}

// ─── Trigger API ──────────────────────────────────────────────────────────────

export interface TriggerAddCronParams {
  trigger_id: string;
  cron_expr: string;
  target_agent: string;
  message?: string;
}

export interface TriggerAddWebhookParams {
  trigger_id: string;
  target_agent: string;
  hmac_secret?: string;
}

export interface TriggerRemoveParams {
  trigger_id: string;
}

// ─── Agent API ────────────────────────────────────────────────────────────────

export interface AgentSpawnConfig {
  system_prompt?: string;
  provider?: string;
  model?: string;
  max_turns?: number;
}

export interface AgentSpawnParams {
  agent_id?: string;
  config: AgentSpawnConfig;
}

export interface AgentKillParams {
  agent_id: string;
}

export interface AgentSteerParams {
  agent_id: string;
  message: string;
}

// ─── Tool API ─────────────────────────────────────────────────────────────────

export interface ToolRegisterParams {
  name: string;
  description: string;
  schema: Record<string, unknown>;
  executor?: string;
}

export interface ToolUnregisterParams {
  name: string;
}

// ─── Skill API ────────────────────────────────────────────────────────────────

export interface SkillLoadDirParams {
  path: string;
}

export interface SkillGetFullParams {
  name: string;
}

// ─── Provider API ─────────────────────────────────────────────────────────────

export interface ProviderRegisterParams {
  name: string;
  provider_type: string;
  api_key?: string;
  base_url?: string;
  model?: string;
}
