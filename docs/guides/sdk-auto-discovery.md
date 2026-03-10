# SDK Auto-Discovery Protocol

This document describes the standard protocol for language SDK clients (Python, TypeScript, Go, etc.) to automatically discover and connect to a running `claw-kernel-server` daemon, or start one if not running.

## Overview

The goal is **zero-configuration for SDK users**: after installing the package, they just write code and the SDK handles daemon lifecycle transparently.

## Platform Socket Paths

The daemon listens on a platform-standard IPC socket:

| Platform | Default Socket Path |
|----------|---------------------|
| Linux | `$XDG_RUNTIME_DIR/claw/kernel.sock` (fallback: `~/.local/share/claw-kernel/kernel.sock`) |
| macOS | `~/Library/Application Support/claw-kernel/kernel.sock` |
| Windows | `\\.\pipe\claw-kernel-<username>` |

You can override this with the `CLAW_SOCKET_PATH` environment variable.

## PID File Paths

The daemon writes its PID to:

| Platform | PID File Path |
|----------|---------------|
| Linux/macOS | `~/.local/share/claw-kernel/kernel.pid` |
| Windows | `%LOCALAPPDATA%\claw-kernel\kernel.pid` |

## Auto-Discovery Algorithm

SDK clients MUST implement the following connection protocol:

```python
def connect_to_kernel():
    socket_path = get_platform_socket_path()  # see table above

    # 1. Try to connect to existing daemon
    if can_connect(socket_path):
        conn = connect(socket_path)
        conn.ping()  # kernel.ping RPC
        return conn

    # 2. Find the daemon binary
    binary = find_binary("claw-kernel-server", search_paths=[
        "$PATH",
        "~/.cargo/bin",
        os.path.dirname(__file__),  # package directory
    ])

    if binary is None:
        raise RuntimeError(
            "claw-kernel-server not found. Install it with:\n"
            "  cargo install claw-kernel"
        )

    # 3. Start the daemon
    subprocess.Popen(
        [binary, "--socket-path", socket_path],
        env=os.environ.copy(),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    # 4. Wait for socket to become available
    wait_for_socket(socket_path, timeout=10.0, interval=0.1)

    # 5. Connect and verify
    conn = connect(socket_path)
    conn.ping()  # kernel.ping -> {"pong": true, "ts": <unix_ms>}
    return conn
```

## Connection Framing

All communication uses **4-byte big-endian length-prefixed JSON-RPC 2.0 frames**:

```
[0..3] uint32 big-endian: payload length in bytes
[4..N] UTF-8 JSON payload
```

## Verification Methods

After connecting, call these methods to verify the daemon:

### `kernel.ping`

```json
{"jsonrpc": "2.0", "method": "kernel.ping", "id": 1}
```

Response:
```json
{"jsonrpc": "2.0", "result": {"pong": true, "ts": 1700000000000}, "id": 1}
```

### `kernel.info`

```json
{"jsonrpc": "2.0", "method": "kernel.info", "id": 1}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": {
    "version": "1.0.0",
    "protocol_version": 1,
    "providers": ["anthropic", "openai", "ollama", "deepseek", "moonshot"],
    "active_provider": "anthropic",
    "active_model": "claude-sonnet-4-6",
    "features": ["streaming", "external_tools"],
    "max_sessions": 16,
    "current_sessions": 0
  },
  "id": 1
}
```

## Session Lifecycle

```
createSession -> sendMessage -> [toolResult*] -> destroySession
```

### Create Session

```json
{
  "jsonrpc": "2.0",
  "method": "createSession",
  "params": {
    "config": {
      "system_prompt": "You are a helpful assistant.",
      "max_turns": 20,
      "provider_override": "openai",
      "model_override": "gpt-4o"
    }
  },
  "id": 1
}
```

### Send Message (returns immediately, streams via notifications)

```json
{
  "jsonrpc": "2.0",
  "method": "sendMessage",
  "params": {
    "session_id": "abc-123",
    "content": "Hello, world!"
  },
  "id": 2
}
```

Streaming notifications pushed to the client:

```json
{"jsonrpc": "2.0", "method": "agent/streamChunk", "params": {"session_id": "abc-123", "delta": "Hello", "done": false}}
{"jsonrpc": "2.0", "method": "agent/finish", "params": {"session_id": "abc-123", "content": "Hello! ...", "reason": "stop"}}
```

## Error Codes

| Code | Name | Description |
|------|------|-------------|
| -32700 | PARSE_ERROR | Invalid JSON |
| -32600 | INVALID_REQUEST | Invalid Request object |
| -32601 | METHOD_NOT_FOUND | Method not available |
| -32602 | INVALID_PARAMS | Invalid parameters |
| -32603 | INTERNAL_ERROR | Internal error |
| -32000 | SESSION_NOT_FOUND | Session does not exist |
| -32001 | MAX_SESSIONS_REACHED | Session limit reached |
| -32002 | PROVIDER_ERROR | LLM provider error |
| -32003 | AGENT_ERROR | Agent loop error |
| -32004 | DAEMON_ALREADY_RUNNING | Another daemon instance is running |
| -32005 | PROVIDER_NOT_FOUND | Provider not registered |

## Reference Implementations

- Python SDK: `pip install claw-kernel` (planned)
- TypeScript SDK: `npm install @claw-project/kernel` (planned)
- Go SDK: `go get github.com/claw-project/claw-kernel-go` (planned)
