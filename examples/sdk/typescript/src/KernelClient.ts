/**
 * claw-kernel TypeScript SDK — Main Client
 *
 * Provides a fully-typed, async/await + AsyncGenerator interface over the
 * claw-kernel IPC daemon.  Zero runtime dependencies — uses Node.js built-ins only.
 *
 * Usage:
 *   const client = await KernelClient.connect();
 *   const sessionId = await client.createSession('You are helpful.');
 *   for await (const token of client.sendMessage(sessionId, 'Hello!')) {
 *     process.stdout.write(token);
 *   }
 *   await client.destroySession(sessionId);
 *   client.close();
 */

import * as fs from 'fs';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';
import { spawn } from 'child_process';

import { FrameBuffer, encodeFrame } from './framing';
import type {
  AgentKillParams,
  AgentSpawnParams,
  AgentSteerParams,
  ChannelInboundParams,
  ChannelRegisterParams,
  ChannelRouteAddParams,
  ChannelRouteRemoveParams,
  ChannelUnregisterParams,
  ChunkParams,
  CreateSessionResult,
  ExternalToolDef,
  FinishParams,
  KernelInfoResult,
  ProviderRegisterParams,
  RpcError,
  RpcNotification,
  RpcResponse,
  ScheduleCancelParams,
  ScheduleCreateParams,
  ScheduleListParams,
  ScheduledTaskInfo,
  SessionConfig,
  SkillGetFullParams,
  SkillLoadDirParams,
  ToolCallParams,
  ToolRegisterParams,
  ToolUnregisterParams,
  TriggerAddCronParams,
  TriggerAddWebhookParams,
  TriggerRemoveParams,
} from './protocol';

// ─── Public types ─────────────────────────────────────────────────────────────

/** A function the client provides to handle LLM tool calls. */
export type ToolHandler = (args: Record<string, unknown>) => unknown | Promise<unknown>;

/** Map of tool name → handler, passed to sendMessage(). */
export type ToolHandlerMap = Record<string, ToolHandler>;

/** Notification listener signature. */
export type NotificationHandler = (method: string, params: unknown) => void;

/** Options accepted by KernelClient.connect(). */
export interface ConnectOptions {
  /** Override the default Unix socket path (or Windows named-pipe path). */
  socketPath?: string;
  /**
   * If true and the daemon is not running, attempt to start it automatically.
   * Requires `claw-kernel-server` to be on PATH.  Default: true.
   */
  autoStart?: boolean;
}

// ─── Internal types ───────────────────────────────────────────────────────────

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
}

// ─── KernelError ─────────────────────────────────────────────────────────────

export class KernelError extends Error {
  constructor(
    public readonly code: number,
    message: string,
    public readonly data?: unknown,
  ) {
    super(`RPC [${code}]: ${message}`);
    this.name = 'KernelError';
  }

  static fromRpcError(err: RpcError): KernelError {
    return new KernelError(err.code, err.message, err.data);
  }
}

// ─── KernelClient ─────────────────────────────────────────────────────────────

export class KernelClient {
  private readonly socket: net.Socket;
  private readonly frameBuffer = new FrameBuffer();
  private reqId = 0;
  private readonly pending = new Map<number, PendingRequest>();
  private readonly notificationListeners: NotificationHandler[] = [];
  private closed = false;

  private constructor(socket: net.Socket) {
    this.socket = socket;
    this._setupReadLoop();
  }

  // ─── Factory ───────────────────────────────────────────────────────────────

  /**
   * Connect to the claw-kernel daemon and authenticate.
   * Starts the daemon automatically if it is not running (unless autoStart is false).
   */
  static async connect(options: ConnectOptions = {}): Promise<KernelClient> {
    const socketPath = options.socketPath ?? KernelClient.defaultSocketPath();
    const autoStart = options.autoStart ?? true;

    // Auto-start daemon if socket doesn't exist
    if (!fs.existsSync(socketPath)) {
      if (!autoStart) {
        throw new Error(
          `claw-kernel daemon not running (socket not found: ${socketPath}). ` +
          'Start it with `claw-kernel-server` or pass autoStart: false to suppress this.',
        );
      }
      KernelClient._startDaemon(socketPath);
    }

    const socket = await KernelClient._connectSocket(socketPath);
    const client = new KernelClient(socket);

    // Auth handshake — first frame must be kernel.auth
    const token = KernelClient._readToken();
    const authResult = await client._call<{ ok: boolean }>('kernel.auth', { token });
    if (!authResult.ok) {
      client.close();
      throw new KernelError(-32001, 'kernel.auth failed: invalid or missing token');
    }

    return client;
  }

  /** Platform-specific default socket path (matches claw-pal dirs). */
  static defaultSocketPath(): string {
    const platform = os.platform();
    if (platform === 'darwin') {
      return path.join(
        os.homedir(), 'Library', 'Application Support', 'claw-kernel', 'kernel.sock',
      );
    }
    if (platform === 'win32') {
      return `\\\\.\\pipe\\claw-kernel-${os.userInfo().username}`;
    }
    // Linux / other Unix
    const xdgRuntime = process.env.XDG_RUNTIME_DIR;
    return xdgRuntime
      ? path.join(xdgRuntime, 'claw', 'kernel.sock')
      : path.join(os.homedir(), '.local', 'share', 'claw-kernel', 'kernel.sock');
  }

  // ─── Read loop ─────────────────────────────────────────────────────────────

  private _setupReadLoop(): void {
    this.socket.on('data', (chunk: Buffer) => {
      let frames: Buffer[];
      try {
        frames = this.frameBuffer.push(chunk);
      } catch (err) {
        this._rejectAll(err instanceof Error ? err : new Error(String(err)));
        this.socket.destroy();
        return;
      }

      for (const frame of frames) {
        this._dispatch(frame);
      }
    });

    this.socket.on('error', (err) => {
      this._rejectAll(err);
    });

    this.socket.on('close', () => {
      this.closed = true;
      this._rejectAll(new Error('Connection closed'));
    });
  }

  private _dispatch(frame: Buffer): void {
    let msg: RpcResponse | RpcNotification;
    try {
      msg = JSON.parse(frame.toString('utf8'));
    } catch (err) {
      // Malformed frame — reject all pending and close
      this._rejectAll(new Error(`Failed to parse frame: ${err}`));
      return;
    }

    // Discriminate: response has a numeric id, notification has no id field.
    if ('id' in msg && msg.id !== null && msg.id !== undefined) {
      const response = msg as RpcResponse;
      const id = typeof response.id === 'number' ? response.id : -1;
      const pending = this.pending.get(id);
      if (pending) {
        this.pending.delete(id);
        if (response.error) {
          pending.reject(KernelError.fromRpcError(response.error));
        } else {
          pending.resolve(response.result ?? {});
        }
      }
    } else {
      // Notification
      const notification = msg as RpcNotification;
      const method = notification.method ?? '';
      const params = notification.params;
      for (const listener of this.notificationListeners) {
        try {
          listener(method, params);
        } catch {
          // Listeners must not throw
        }
      }
    }
  }

  private _rejectAll(err: Error): void {
    for (const pending of this.pending.values()) {
      pending.reject(err);
    }
    this.pending.clear();
  }

  // ─── Low-level RPC ─────────────────────────────────────────────────────────

  private _call<T = unknown>(method: string, params?: unknown): Promise<T> {
    if (this.closed) {
      return Promise.reject(new Error('KernelClient is closed'));
    }

    const id = ++this.reqId;
    const payload = params !== undefined
      ? { jsonrpc: '2.0', method, params, id }
      : { jsonrpc: '2.0', method, id };

    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (v) => resolve(v as T),
        reject,
      });
      this.socket.write(encodeFrame(payload));
    });
  }

  /** Register a raw notification listener. Remove it with removeNotificationListener(). */
  addNotificationListener(handler: NotificationHandler): void {
    this.notificationListeners.push(handler);
  }

  /** Remove a previously added notification listener. */
  removeNotificationListener(handler: NotificationHandler): void {
    const idx = this.notificationListeners.indexOf(handler);
    if (idx !== -1) this.notificationListeners.splice(idx, 1);
  }

  // ─── Session API ───────────────────────────────────────────────────────────

  /**
   * Create a new agent session.
   *
   * @param systemPrompt  System prompt for the session.
   * @param options       Additional session configuration.
   * @returns Session ID string.
   */
  async createSession(
    systemPrompt = '',
    options: Omit<SessionConfig, 'system_prompt'> = {},
  ): Promise<string> {
    const config: SessionConfig = { ...options };
    if (systemPrompt) config.system_prompt = systemPrompt;

    const result = await this._call<CreateSessionResult>('createSession', { config });
    return result.session_id;
  }

  /**
   * Send a message and stream the response token by token.
   *
   * This is an AsyncGenerator — iterate with `for await`:
   *
   *   for await (const token of client.sendMessage(sessionId, 'Hello')) {
   *     process.stdout.write(token);
   *   }
   *
   * When `tools` is provided, the client automatically handles `agent/toolCall`
   * notifications by invoking the matching handler and sending `toolResult` back
   * before the generator yields the next chunk.
   *
   * @param sessionId  Target session.
   * @param content    User message.
   * @param tools      Optional tool handler map.
   */
  async *sendMessage(
    sessionId: string,
    content: string,
    tools?: ToolHandlerMap,
  ): AsyncGenerator<string, void, undefined> {
    // Queue of deltas (strings), or null as end-of-stream sentinel, or Error.
    const queue: Array<string | null | Error> = [];
    let wakeup: (() => void) | null = null;

    const push = (item: string | null | Error) => {
      queue.push(item);
      wakeup?.();
      wakeup = null;
    };

    const handler: NotificationHandler = (method, params) => {
      const p = params as Record<string, unknown>;

      // Filter notifications that don't belong to our session
      if (p?.session_id !== sessionId) return;

      if (method === 'agent/streamChunk') {
        const chunk = p as unknown as ChunkParams;
        if (chunk.delta) push(chunk.delta);
        if (chunk.done) push(null);
      } else if (method === 'agent/toolCall') {
        const tc = p as unknown as ToolCallParams;
        // Execute the tool asynchronously and send toolResult.
        // The kernel waits for toolResult before sending more chunks, so
        // protocol ordering is guaranteed without any extra coordination.
        void this._handleToolCall(sessionId, tc, tools ?? {});
      } else if (method === 'agent/finish') {
        const fin = p as unknown as FinishParams;
        if (fin.content) push(fin.content);
        push(null);
      }
    };

    this.addNotificationListener(handler);

    try {
      // Trigger the agent loop on the daemon side
      await this._call('sendMessage', { session_id: sessionId, content });

      // Drain the queue
      while (true) {
        if (queue.length === 0) {
          // Wait until the handler pushes something
          await new Promise<void>((resolve) => { wakeup = resolve; });
        }

        const item = queue.shift()!;
        if (item === null) break;
        if (item instanceof Error) throw item;
        yield item;
      }
    } finally {
      this.removeNotificationListener(handler);
    }
  }

  private async _handleToolCall(
    sessionId: string,
    tc: ToolCallParams,
    tools: ToolHandlerMap,
  ): Promise<void> {
    const handler = tools[tc.tool_name];
    let result: unknown;
    let success: boolean;

    if (handler) {
      try {
        result = await handler(tc.arguments);
        success = true;
      } catch (err) {
        result = String(err);
        success = false;
      }
    } else {
      result = `Unknown tool: ${tc.tool_name}`;
      success = false;
    }

    try {
      await this._call('toolResult', {
        session_id: sessionId,
        tool_call_id: tc.tool_call_id,
        result,
        success,
      });
    } catch {
      // toolResult failure is non-fatal for the client
    }
  }

  /** Destroy a session and release server-side resources. */
  async destroySession(sessionId: string): Promise<void> {
    await this._call('destroySession', { session_id: sessionId });
  }

  // ─── kernel.info ───────────────────────────────────────────────────────────

  /** Retrieve kernel version, provider list, and feature flags. */
  async info(): Promise<KernelInfoResult> {
    return this._call<KernelInfoResult>('kernel.info');
  }

  // ─── Tool API ──────────────────────────────────────────────────────────────

  /** Register a global tool in the kernel's tool registry. */
  async registerTool(params: ToolRegisterParams): Promise<void> {
    await this._call('tool.register', params);
  }

  /** Unregister a previously registered tool. */
  async unregisterTool(name: string): Promise<void> {
    await this._call('tool.unregister', { name } satisfies ToolUnregisterParams);
  }

  /** List all registered tools. */
  async listTools(): Promise<ExternalToolDef[]> {
    return this._call<ExternalToolDef[]>('tool.list');
  }

  // ─── Skill API ─────────────────────────────────────────────────────────────

  /** Load all skills from a filesystem directory. */
  async skillLoadDir(dirPath: string): Promise<void> {
    await this._call('skill.load_dir', { path: dirPath } satisfies SkillLoadDirParams);
  }

  /** Get the full definition of a named skill. */
  async skillGetFull(name: string): Promise<unknown> {
    return this._call('skill.get_full', { name } satisfies SkillGetFullParams);
  }

  /** List all loaded skills. */
  async listSkills(): Promise<string[]> {
    return this._call<string[]>('skill.list');
  }

  // ─── Provider API ──────────────────────────────────────────────────────────

  /** Dynamically register an LLM provider at runtime. */
  async registerProvider(params: ProviderRegisterParams): Promise<void> {
    await this._call('provider.register', params);
  }

  // ─── Agent API ─────────────────────────────────────────────────────────────

  /** Spawn a new autonomous agent. */
  async spawnAgent(params: AgentSpawnParams): Promise<{ agent_id: string }> {
    return this._call('agent.spawn', params);
  }

  /** Terminate a running agent. */
  async killAgent(agentId: string): Promise<void> {
    await this._call('agent.kill', { agent_id: agentId } satisfies AgentKillParams);
  }

  /** Inject a steering message into a running agent. */
  async steerAgent(agentId: string, message: string): Promise<void> {
    await this._call('agent.steer', { agent_id: agentId, message } satisfies AgentSteerParams);
  }

  /** List all active agent IDs. */
  async listAgents(): Promise<string[]> {
    return this._call<string[]>('agent.list');
  }

  // ─── Channel API ───────────────────────────────────────────────────────────

  /** Register an external channel (webhook, discord, etc.). */
  async registerChannel(params: ChannelRegisterParams): Promise<void> {
    await this._call('channel.register', params);
  }

  /** Unregister a channel. */
  async unregisterChannel(channelId: string): Promise<void> {
    await this._call('channel.unregister', { channel_id: channelId } satisfies ChannelUnregisterParams);
  }

  /** List all registered channels. */
  async listChannels(): Promise<unknown[]> {
    return this._call<unknown[]>('channel.list');
  }

  /** Submit an inbound message from an external channel. */
  async channelInbound(params: ChannelInboundParams): Promise<void> {
    await this._call('channel.inbound', params);
  }

  /** Add a routing rule to the channel router. */
  async addChannelRoute(params: ChannelRouteAddParams): Promise<void> {
    await this._call('channel.route_add', params);
  }

  /** Remove routing rules for an agent. */
  async removeChannelRoute(agentId: string): Promise<void> {
    await this._call('channel.route_remove', { agent_id: agentId } satisfies ChannelRouteRemoveParams);
  }

  // ─── Trigger API ───────────────────────────────────────────────────────────

  /** Add a cron trigger for an agent. */
  async addCronTrigger(params: TriggerAddCronParams): Promise<void> {
    await this._call('trigger.add_cron', params);
  }

  /** Add a webhook trigger for an agent. */
  async addWebhookTrigger(params: TriggerAddWebhookParams): Promise<void> {
    await this._call('trigger.add_webhook', params);
  }

  /** Remove a trigger by ID. */
  async removeTrigger(triggerId: string): Promise<void> {
    await this._call('trigger.remove', { trigger_id: triggerId } satisfies TriggerRemoveParams);
  }

  // ─── Schedule API ──────────────────────────────────────────────────────────

  /** Create a scheduled task. */
  async createSchedule(params: ScheduleCreateParams): Promise<{ task_id: string }> {
    return this._call('schedule.create', params);
  }

  /** Cancel a scheduled task. */
  async cancelSchedule(taskId: string): Promise<void> {
    await this._call('schedule.cancel', { task_id: taskId } satisfies ScheduleCancelParams);
  }

  /** List scheduled tasks for a session. */
  async listSchedules(sessionId: string): Promise<ScheduledTaskInfo[]> {
    return this._call<ScheduledTaskInfo[]>('schedule.list', { session_id: sessionId } satisfies ScheduleListParams);
  }

  // ─── Lifecycle ─────────────────────────────────────────────────────────────

  /** Close the socket. Outstanding pending calls will be rejected. */
  close(): void {
    if (!this.closed) {
      this.closed = true;
      this.socket.destroy();
    }
  }

  // ─── Static helpers ────────────────────────────────────────────────────────

  private static _dataDir(): string {
    const platform = os.platform();
    if (platform === 'darwin') {
      return path.join(os.homedir(), 'Library', 'Application Support', 'claw-kernel');
    }
    if (platform === 'win32') {
      const localAppData =
        process.env.LOCALAPPDATA ??
        path.join(os.homedir(), 'AppData', 'Local');
      return path.join(localAppData, 'claw-kernel');
    }
    const xdg = process.env.XDG_RUNTIME_DIR;
    return xdg
      ? path.join(xdg, 'claw')
      : path.join(os.homedir(), '.local', 'share', 'claw-kernel');
  }

  private static _readToken(): string {
    const tokenPath = path.join(KernelClient._dataDir(), 'kernel.token');
    try {
      return fs.readFileSync(tokenPath, 'utf8').trim();
    } catch {
      return '';
    }
  }

  private static _startDaemon(socketPath: string): void {
    // Detached spawn — daemon outlives this process
    let child;
    try {
      child = spawn('claw-kernel-server', ['--socket-path', socketPath], {
        detached: true,
        stdio: 'ignore',
      });
    } catch {
      throw new Error(
        'claw-kernel-server not found in PATH. ' +
        'Install it first or pass { autoStart: false } to connect().',
      );
    }
    // Let the daemon keep running after we exit
    child.unref();

    // Poll until socket appears (max 3 s, 100 ms intervals)
    const deadline = Date.now() + 3000;
    while (Date.now() < deadline) {
      if (fs.existsSync(socketPath)) return;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
    }

    throw new Error(
      `claw-kernel-server was started but socket did not appear at ${socketPath} within 3 s.`,
    );
  }

  private static _connectSocket(socketPath: string): Promise<net.Socket> {
    return new Promise((resolve, reject) => {
      const socket = net.createConnection(socketPath);

      const onConnect = () => {
        socket.removeListener('error', onError);
        resolve(socket);
      };
      const onError = (err: Error) => {
        socket.removeListener('connect', onConnect);
        reject(err);
      };

      socket.once('connect', onConnect);
      socket.once('error', onError);
    });
  }
}
