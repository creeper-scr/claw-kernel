# V8/TypeScript Script Examples

This directory contains example scripts for the claw-kernel V8 JavaScript/TypeScript engine.

## Prerequisites

Enable the `engine-v8` feature when building claw-script:

```toml
[dependencies]
claw-script = { version = "0.1", features = ["engine-v8"] }
```

## Examples

### hello.js

Basic JavaScript example demonstrating:
- Global `agent_id` access
- `claw.events.emit()` for event publishing
- `claw.dirs.*` for directory paths
- Returning JSON objects

```bash
# Run via your claw-kernel application
cargo run --features engine-v8 --example v8_runner -- examples/v8-scripts/hello.js
```

### data-processor.ts

TypeScript example demonstrating:
- Interface definitions
- Type-safe functions
- Array processing with `.map()`
- Async/await patterns
- Event emission

```bash
# Run via your claw-kernel application
cargo run --features engine-v8 --example v8_runner -- examples/v8-scripts/data-processor.ts
```

## Available APIs

All scripts have access to the `claw.*` namespace:

### claw.fs
- `read(path: string): Promise<Uint8Array>`
- `write(path: string, data: Uint8Array): Promise<void>`
- `exists(path: string): boolean`
- `listDir(path: string): string[]`
- `glob(pattern: string): string[]`

### claw.net
- `get(url: string, headers?: object): Promise<Response>`
- `post(url: string, headers: object, body: string): Promise<Response>`

### claw.tools
- `call(name: string, params: object): Promise<any>`
- `list(): ToolInfo[]`
- `exists(name: string): boolean`

### claw.memory
- `get(key: string): Promise<any>`
- `set(key: string, value: any): Promise<void>`
- `delete(key: string): Promise<void>`
- `search(query: string, topK: number): Promise<MemoryItem[]>`

### claw.events
- `emit(event: string, data: any): void`
- `on(event: string, handler: (data: any) => void): void`
- `once(event: string, handler: (data: any) => void): void`
- `poll(): void`

### claw.agent
- `spawn(name: string): AgentId`
- `status(id: AgentId): string`
- `kill(id: AgentId): void`
- `list(): AgentId[]`
- `info(id: AgentId): AgentInfo | null`

### claw.dirs
- `configDir(): string`
- `dataDir(): string`
- `cacheDir(): string`
- `toolsDir(): string`
- `scriptsDir(): string`
- `logsDir(): string`

### claw.json
- `parse(text: string): any`
- `stringify(value: any, opts?: { pretty?: boolean }): string`

## Globals

- `agent_id: string` - The ID of the executing agent
- `console` - Console object for logging (output goes to Rust tracing)

## TypeScript Support

TypeScript is automatically transpiled to JavaScript when using `Script::typescript()`:

```rust
let script = Script::typescript("my-tool", r#"
    interface Config {
        name: string;
        value: number;
    }
    
    const cfg: Config = { name: "test", value: 42 };
    return cfg;
"#);
```

## Security

- Each script execution runs in a fresh V8 isolate
- Memory limits are enforced (default: 128MB)
- Execution timeout is enforced (default: 30s)
- Scripts cannot access Rust kernel internals directly

## More Information

- [claw-script documentation](../../docs/crates/claw-script.md)
- [ADR-012: V8 Engine Implementation](../../docs/adr/012-v8-engine-implementation.md)
- [V8 documentation](https://v8.dev/docs)
