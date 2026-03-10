# claw-kernel TypeScript SDK

TypeScript/Node.js SDK for the claw-kernel IPC daemon.

**Requirements**: Node.js 14+ · Zero runtime dependencies

## Quick Start

```bash
cd sdk/typescript
npm install
npx ts-node examples/basic-chat.ts
```

## 10-Line Example

```typescript
import { KernelClient } from './src';

const client = await KernelClient.connect();
const sessionId = await client.createSession('You are helpful.');

for await (const token of client.sendMessage(sessionId, 'Hello!')) {
  process.stdout.write(token);
}

await client.destroySession(sessionId);
client.close();
```

## Architecture

```
sdk/typescript/
├── src/
│   ├── framing.ts       4-byte BE length-prefixed frame layer
│   ├── protocol.ts      TypeScript types mirroring protocol.rs
│   ├── KernelClient.ts  Main client class
│   └── index.ts         Public re-exports
└── examples/
    ├── basic-chat.ts        Simple Q&A with streaming
    └── tool-registration.ts Tool callbacks (weather + calculator)
```

## Wire Protocol

The IPC protocol is JSON-RPC 2.0 over a Unix domain socket (or Windows named pipe).

Each message is length-prefixed:

```
┌──────────────────┬──────────────────────────┐
│  4 bytes BE u32  │  UTF-8 JSON payload       │
└──────────────────┴──────────────────────────┘
```

Maximum frame size: **16 MiB**.

The client must send `kernel.auth` as its very first request:

```json
{ "jsonrpc": "2.0", "method": "kernel.auth", "params": { "token": "<token>" }, "id": 1 }
```

The token is read from `~/.local/share/claw-kernel/kernel.token` (Linux),
`~/Library/Application Support/claw-kernel/kernel.token` (macOS), or
`%LOCALAPPDATA%\claw-kernel\kernel.token` (Windows).

## API Reference

### `KernelClient.connect(options?)`

```typescript
const client = await KernelClient.connect({
  socketPath: '/tmp/custom.sock', // optional override
  autoStart: true,                // auto-start daemon if not running (default: true)
});
```

### `createSession(systemPrompt?, options?)`

```typescript
const sessionId = await client.createSession('You are helpful.', {
  model_override: 'claude-opus-4-6',
  max_turns: 30,
  tools: [{ name: 'get_weather', description: '...', input_schema: { ... } }],
});
```

### `sendMessage(sessionId, content, tools?)` — AsyncGenerator

```typescript
for await (const token of client.sendMessage(sessionId, 'Hello', {
  get_weather: async (args) => fetchWeather(args.city as string),
})) {
  process.stdout.write(token);
}
```

Tool handlers receive the `arguments` object from the LLM and must return a
value (sync or async). The SDK handles the `toolResult` round-trip automatically.

### Other Methods

| Method | Description |
|--------|-------------|
| `info()` | Kernel version, providers, features |
| `destroySession(id)` | Release session resources |
| `registerTool(params)` | Register global tool |
| `spawnAgent(params)` | Spawn autonomous agent |
| `killAgent(id)` | Stop agent |
| `steerAgent(id, msg)` | Inject steering message |
| `registerChannel(params)` | Register external channel |
| `channelInbound(params)` | Submit inbound message |
| `addCronTrigger(params)` | Schedule cron trigger |
| `addWebhookTrigger(params)` | Register webhook trigger |
| `createSchedule(params)` | Schedule a task |
| `registerProvider(params)` | Register LLM provider |
| `addNotificationListener(fn)` | Raw notification hook |

### Error Handling

```typescript
import { KernelError } from './src';

try {
  await client.sendMessage(sessionId, '...').next();
} catch (err) {
  if (err instanceof KernelError) {
    console.error(`RPC [${err.code}]: ${err.message}`);
  }
}
```

## Symmetry with Python SDK

| Feature | Python SDK | TypeScript SDK |
|---------|-----------|----------------|
| Transport | `socket.AF_UNIX` | `net.Socket` |
| Frame layer | `struct.pack(">I", ...)` | `Buffer.writeUInt32BE(...)` |
| Auth | `_call("kernel.auth", ...)` | `_call("kernel.auth", ...)` |
| Streaming | `Iterator[str]` (generator) | `AsyncGenerator<string>` |
| Tool handling | Sync `_call("toolResult")` | `await _call("toolResult")` |
| Auto-start | `subprocess.Popen(...)` | `spawnSync(...)` |
