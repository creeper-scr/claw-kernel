import { KernelClient } from './client';
import type { StreamChunk, ToolDef } from './types';

/**
 * Handle for an active agent session.
 *
 * Obtained via `KernelClient.createSession()`. Provides `stream()` for
 * async-iterable streaming responses and `send()` for collecting the full reply.
 *
 * Client-side tools registered at session creation are executed locally:
 * the kernel sends `agent/toolCall` notifications; this class dispatches them
 * and sends results back via `toolResult` RPC calls.
 */
export class SessionHandle {
  readonly id: string;
  private client: KernelClient;
  private tools: Map<string, ToolDef['execute']>;

  constructor(id: string, client: KernelClient, tools: ToolDef[] = []) {
    this.id = id;
    this.client = client;
    this.tools = new Map(tools.map(t => [t.name, t.execute]));
  }

  /**
   * Handle an incoming `agent/toolCall` notification for this session.
   * Executes the local tool executor and sends the result back to the kernel.
   */
  private async _handleToolCall(
    toolCallId: string,
    toolName: string,
    args: Record<string, unknown>
  ): Promise<void> {
    const executor = this.tools.get(toolName);
    let result: unknown;
    let success: boolean;

    if (executor) {
      try {
        result = await executor(args);
        success = true;
      } catch (err) {
        result = err instanceof Error ? err.message : String(err);
        success = false;
      }
    } else {
      result = `Unknown tool: ${toolName}`;
      success = false;
    }

    await this.client.call('toolResult', {
      session_id: this.id,
      tool_call_id: toolCallId,
      result,
      success,
    });
  }

  /**
   * Send a message and return an async generator of `StreamChunk` objects.
   *
   * The generator yields:
   * - `{ type: 'delta', delta: string }` — incremental text from the model
   * - `{ type: 'toolCall', ... }` — tool invocation (handled automatically)
   * - `{ type: 'finish', finishReason: string }` — stream end
   *
   * Tool calls are handled transparently: `agent/toolCall` notifications trigger
   * local execution and the result is sent back without interrupting the generator.
   *
   * @example
   * for await (const chunk of session.stream('Hello!')) {
   *   if (chunk.delta) process.stdout.write(chunk.delta);
   * }
   */
  async *stream(message: string): AsyncGenerator<StreamChunk> {
    // Simple queue + waiter pattern — avoids external dependencies
    const queue: StreamChunk[] = [];
    let done = false;
    let streamError: Error | undefined;
    const resolvers: Array<() => void> = [];

    const enqueue = (chunk: StreamChunk): void => {
      queue.push(chunk);
      resolvers.shift()?.();
    };

    const finish = (err?: Error): void => {
      done = true;
      streamError = err;
      resolvers.shift()?.();
    };

    // --- Notification handlers ---

    const chunkHandler = (params: unknown): void => {
      const p = params as { session_id: string; delta?: string; done?: boolean };
      if (p.session_id !== this.id) return;
      if (p.delta) {
        enqueue({ type: 'delta', delta: p.delta });
      }
      if (p.done) {
        enqueue({ type: 'finish', finishReason: 'stop' });
        finish();
      }
    };

    const finishHandler = (params: unknown): void => {
      const p = params as { session_id: string; finish_reason?: string };
      if (p.session_id !== this.id) return;
      enqueue({ type: 'finish', finishReason: p.finish_reason ?? 'stop' });
      finish();
    };

    const toolCallHandler = (params: unknown): void => {
      const p = params as {
        session_id: string;
        tool_call_id: string;
        tool_name: string;
        arguments: Record<string, unknown>;
      };
      if (p.session_id !== this.id) return;
      enqueue({
        type: 'toolCall',
        toolCallId: p.tool_call_id,
        toolName: p.tool_name,
        toolInput: p.arguments,
      });
      // Execute tool asynchronously without blocking the stream
      this._handleToolCall(p.tool_call_id, p.tool_name, p.arguments).catch(() => {
        // Errors are reported to kernel via toolResult; don't crash the stream
      });
    };

    this.client.onNotification('agent/streamChunk', chunkHandler);
    this.client.onNotification('agent/finish', finishHandler);
    this.client.onNotification('agent/toolCall', toolCallHandler);

    try {
      // `sendMessage` uses `content` as the param key (confirmed from Python SDK)
      await this.client.call('sendMessage', {
        session_id: this.id,
        content: message,
      });

      // Yield chunks as they arrive
      while (!done || queue.length > 0) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else if (!done) {
          await new Promise<void>(r => resolvers.push(r));
        }
      }

      if (streamError) throw streamError;
    } finally {
      this.client.offNotification('agent/streamChunk', chunkHandler);
      this.client.offNotification('agent/finish', finishHandler);
      this.client.offNotification('agent/toolCall', toolCallHandler);
    }
  }

  /**
   * Send a message and collect the full text response as a string.
   * Convenience wrapper around `stream()`.
   */
  async send(message: string): Promise<string> {
    let fullText = '';
    for await (const chunk of this.stream(message)) {
      if (chunk.delta) fullText += chunk.delta;
    }
    return fullText;
  }

  /**
   * Destroy this session on the kernel, freeing server-side resources.
   */
  async close(): Promise<void> {
    await this.client.call('destroySession', { session_id: this.id });
  }
}
