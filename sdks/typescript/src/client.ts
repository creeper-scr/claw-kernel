import * as net from 'net';
import { EventEmitter } from 'events';
import { encodeFrame, FrameReader } from './framing';
import { discoverSocketPath, readAuthToken } from './auto-discovery';
import { SessionHandle } from './session';
import type {
  ConnectOptions,
  KernelInfo,
  RpcRequest,
  RpcResponse,
  RpcNotification,
  ToolDef,
} from './types';

/** Pending RPC call waiting for response */
interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
}

/**
 * KernelClient — connects to the claw-kernel IPC daemon.
 *
 * Frame format: 4-byte Big Endian length prefix + UTF-8 JSON payload.
 * Authentication: first RPC call must be `kernel.auth` with the token read
 * from `~/.local/share/claw-kernel/kernel.token` (or platform equivalent).
 *
 * @example
 * const client = await KernelClient.connect();
 * const session = await client.createSession({ systemPrompt: 'You are helpful.' });
 * for await (const chunk of session.stream('Hello!')) {
 *   process.stdout.write(chunk.delta ?? '');
 * }
 * await client.close();
 */
export class KernelClient extends EventEmitter {
  private socket!: net.Socket;
  private pending: Map<number | string, PendingCall> = new Map();
  private nextId = 1;
  private socketPath: string;
  private token: string;
  private closed = false;
  /** Notification listeners: method -> callbacks */
  private notificationHandlers: Map<string, Array<(params: unknown) => void>> = new Map();

  private constructor(socketPath: string, token: string) {
    super();
    this.socketPath = socketPath;
    this.token = token;
  }

  /**
   * Connect to the claw-kernel daemon.
   *
   * Automatically discovers socket path and reads auth token from the
   * platform data directory. Override via `options.socketPath` / `options.token`.
   *
   * @throws {Error} if connection fails after all retry attempts or auth is rejected.
   */
  static async connect(options: ConnectOptions = {}): Promise<KernelClient> {
    const socketPath = options.socketPath ?? discoverSocketPath();
    const token = options.token ?? readAuthToken() ?? '';

    const client = new KernelClient(socketPath, token);
    await client._connect(options.reconnectAttempts ?? 5, options.reconnectDelayMs ?? 500);
    await client._authenticate();
    return client;
  }

  private async _connect(retries: number, delayMs: number): Promise<void> {
    for (let attempt = 0; attempt <= retries; attempt++) {
      try {
        await this._doConnect();
        return;
      } catch (err) {
        if (attempt === retries) throw err;
        await new Promise(r => setTimeout(r, delayMs * Math.pow(2, attempt)));
      }
    }
  }

  private _doConnect(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.socket = net.createConnection(this.socketPath);
      const reader = new FrameReader((frame) => this._handleFrame(frame));

      this.socket.on('data', (chunk: Buffer) => reader.feed(chunk));
      this.socket.once('connect', () => resolve());
      this.socket.once('error', (err) => reject(err));
      this.socket.on('close', () => {
        this.closed = true;
        this.emit('close');
        // Reject all pending calls so callers don't hang indefinitely
        for (const [, pending] of this.pending) {
          pending.reject(new Error('Connection closed'));
        }
        this.pending.clear();
      });
    });
  }

  /**
   * Perform the kernel.auth handshake.
   * The server expects `{ok: true}` in the result on success.
   */
  private async _authenticate(): Promise<void> {
    const result = await this.call('kernel.auth', { token: this.token }) as { ok?: boolean };
    if (result.ok === false) {
      throw new Error('kernel.auth failed: invalid token');
    }
  }

  private _handleFrame(frame: Buffer): void {
    let msg: RpcResponse | RpcNotification;
    try {
      msg = JSON.parse(frame.toString('utf8'));
    } catch {
      this.emit('error', new Error('Failed to parse frame as JSON'));
      return;
    }

    // Notification: no `id` field (or id is undefined/null)
    const asAny = msg as Record<string, unknown>;
    if (!('id' in asAny) || asAny['id'] === undefined || asAny['id'] === null) {
      const notif = msg as RpcNotification;
      const handlers = this.notificationHandlers.get(notif.method) ?? [];
      for (const h of handlers) h(notif.params);
      this.emit('notification', notif);
      return;
    }

    // Response: has an `id` matching a pending call
    const response = msg as RpcResponse;
    const pending = this.pending.get(response.id!);
    if (pending) {
      this.pending.delete(response.id!);
      if (response.error) {
        pending.reject(
          new Error(`RPC error ${response.error.code}: ${response.error.message}`)
        );
      } else {
        pending.resolve(response.result);
      }
    }
  }

  /**
   * Make a JSON-RPC 2.0 call and await the response.
   */
  call(method: string, params?: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      if (this.closed) {
        reject(new Error('Client is closed'));
        return;
      }
      const id = this.nextId++;
      const request: RpcRequest = {
        jsonrpc: '2.0',
        method,
        params,
        id,
      };
      this.pending.set(id, { resolve, reject });
      this.socket.write(encodeFrame(request));
    });
  }

  /**
   * Subscribe to notifications for a given RPC method name.
   */
  onNotification(method: string, handler: (params: unknown) => void): void {
    if (!this.notificationHandlers.has(method)) {
      this.notificationHandlers.set(method, []);
    }
    this.notificationHandlers.get(method)!.push(handler);
  }

  /**
   * Remove a previously registered notification handler.
   */
  offNotification(method: string, handler: (params: unknown) => void): void {
    const handlers = this.notificationHandlers.get(method);
    if (handlers) {
      const idx = handlers.indexOf(handler);
      if (idx !== -1) handlers.splice(idx, 1);
    }
  }

  /**
   * Create a new agent session on the kernel.
   *
   * Pass `tools` to register client-side tool executors that the agent can invoke.
   * Tool schema is sent to the kernel; execution happens locally in this process.
   */
  async createSession(config: {
    systemPrompt?: string;
    maxTurns?: number;
    tokenBudget?: number;
    model?: string;
    tools?: ToolDef[];
  } = {}): Promise<SessionHandle> {
    // Build the server-side config (schema only, no executor functions)
    const toolDefs = config.tools?.map(t => ({
      name: t.name,
      description: t.description,
      input_schema: t.inputSchema,
    }));

    const serverConfig: Record<string, unknown> = {};
    if (config.systemPrompt !== undefined) serverConfig['system_prompt'] = config.systemPrompt;
    if (config.maxTurns !== undefined) serverConfig['max_turns'] = config.maxTurns;
    if (config.tokenBudget !== undefined) serverConfig['token_budget'] = config.tokenBudget;
    if (config.model !== undefined) serverConfig['model_override'] = config.model;
    if (toolDefs !== undefined) serverConfig['tools'] = toolDefs;

    const result = await this.call('createSession', { config: serverConfig }) as {
      session_id: string;
    };

    return new SessionHandle(result.session_id, this, config.tools ?? []);
  }

  /**
   * Get kernel info (version, protocol version, active session count).
   */
  async info(): Promise<KernelInfo> {
    const result = await this.call('kernel.info') as {
      version: string;
      protocol_version: number;
      active_sessions: number;
    };
    return {
      version: result.version,
      protocolVersion: result.protocol_version,
      activeSessions: result.active_sessions,
    };
  }

  /**
   * Ping the kernel server. Returns true on success, false on failure.
   */
  async ping(): Promise<boolean> {
    try {
      await this.call('kernel.ping');
      return true;
    } catch {
      return false;
    }
  }

  /** Skill management methods */
  readonly skills = {
    loadDir: (dirPath: string) =>
      this.call('skill.load_dir', { path: dirPath }) as Promise<{
        path: string;
        count: number;
      }>,

    list: () =>
      this.call('skill.list') as Promise<Array<{ name: string; description: string }>>,

    getFull: (name: string) =>
      this.call('skill.get_full', { name }) as Promise<{ name: string; content: string }>,
  };

  /** Tool management (global server-side registration) */
  readonly tools = {
    register: (name: string, description: string, schema: Record<string, unknown>) =>
      this.call('tool.register', { name, description, schema }),

    unregister: (name: string) =>
      this.call('tool.unregister', { name }),

    list: () =>
      this.call('tool.list') as Promise<
        Array<{ name: string; description: string; schema: unknown }>
      >,
  };

  /**
   * Close the connection. Any pending calls will be rejected.
   */
  close(): void {
    this.closed = true;
    this.socket.destroy();
  }
}
