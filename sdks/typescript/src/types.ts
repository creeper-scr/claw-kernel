/** JSON-RPC 2.0 request */
export interface RpcRequest {
  jsonrpc: '2.0';
  method: string;
  params?: unknown;
  id?: number | string;
}

/** JSON-RPC 2.0 response */
export interface RpcResponse {
  jsonrpc: '2.0';
  result?: unknown;
  error?: RpcError;
  id?: number | string;
}

/** JSON-RPC 2.0 notification (no id) */
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

/** Session configuration */
export interface SessionConfig {
  systemPrompt?: string;
  maxTurns?: number;
  tokenBudget?: number;
  model?: string;
  tools?: ToolDef[];
}

/** Tool definition */
export interface ToolDef {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  execute: (args: Record<string, unknown>) => Promise<unknown>;
}

/** Streaming chunk from agent */
export interface StreamChunk {
  type: 'delta' | 'toolCall' | 'toolResult' | 'finish';
  delta?: string;
  toolCallId?: string;
  toolName?: string;
  toolInput?: unknown;
  finishReason?: string;
}

/** Connection options */
export interface ConnectOptions {
  socketPath?: string;
  token?: string;
  reconnectAttempts?: number;
  reconnectDelayMs?: number;
}

/** Kernel info */
export interface KernelInfo {
  version: string;
  protocolVersion: number;
  activeSessions: number;
}
