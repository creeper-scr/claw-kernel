export { KernelClient } from './client';
export { SessionHandle } from './session';
export { discoverSocketPath, readAuthToken } from './auto-discovery';
export type {
  ConnectOptions,
  SessionConfig,
  ToolDef,
  StreamChunk,
  KernelInfo,
  RpcRequest,
  RpcResponse,
  RpcNotification,
  RpcError,
} from './types';
