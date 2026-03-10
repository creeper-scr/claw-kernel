---
title: "ADR-011: Multi-Language Support via IPC Daemon (KernelServer)"
description: "Non-Rust applications access the kernel through a local IPC daemon rather than a Rust library dependency"
status: accepted
accepted_date: 2026-03-08
date: 2026-03-08
type: adr
---

# ADR-011: Multi-Language Support via IPC Daemon (KernelServer)

**Status:** Accepted
**Date:** 2026-03-08
**Deciders:** claw-kernel core team

---

## Context

### The original goal

claw-kernel exists to eliminate repeated reimplementation across the Claw ecosystem (OpenClaw, ZeroClaw, PicoClaw, …). Every project independently reimplements LLM HTTP calls, tool dispatch, agent loop management, and context handling.

The kernel provides all of this in Rust. However, the current integration model requires application code to also be written in Rust:

```toml
# Only works for Rust projects
[dependencies]
claw-kernel = { git = "...", features = ["engine-lua"] }
```

Projects written in TypeScript, Python, Go, or any other language cannot benefit from the shared kernel.

### Two distinct extension scenarios

This ADR separates two scenarios that must not be conflated:

| Scenario | Who writes the code | Language | Purpose |
|----------|---------------------|----------|---------|
| **Agent self-evolution** | The agent itself, at runtime | Lua / JS (embedded) | Agent generates and executes new tools/behaviors |
| **Application development** | Human developers | Any language | Build Openclaw-equivalent applications |

**Embedded script engines (`claw-script`) address scenario 1 only.** Lua and V8 run agent-generated code inside the kernel process. They are not the right mechanism for building full applications in other languages.

### Why embedded scripts are insufficient for scenario 2

- Sandboxed — limited system access
- Stateless — no persistent connections or caches
- In-process — share the kernel's memory space and failure domain
- Three languages only — excludes Go, Java, Ruby, Kotlin, Swift, …

---

## Decision

**Add a `KernelServer` that exposes the full agent loop via local IPC (Unix Domain Socket on macOS/Linux, Named Pipe on Windows), using a JSON-RPC protocol over the existing `claw-pal` framing layer.**

Non-Rust applications become thin IPC clients. The kernel daemon handles all AI infrastructure.

---

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│  Application layer (any language)                          │
│  Python · TypeScript · Go · Java · Ruby · …                │
├────────────────┬───────────────────────────────────────────┤
│  Thin client   │  ~50–100 lines per language               │
│  SDK           │  JSON-RPC over Unix socket                │
├────────────────┴───────────────────────────────────────────┤
│                Unix Domain Socket                          │
│         (claw-pal framing: 4-byte BE length prefix)        │
├────────────────────────────────────────────────────────────┤
│  KernelServer (new crate: claw-server)                     │
│    ├── Session manager                                     │
│    ├── AgentLoop dispatch                                  │
│    └── Tool call callback routing                          │
├────────────────────────────────────────────────────────────┤
│  claw-kernel core (unchanged)                              │
│  Provider · ToolRegistry · AgentLoop · Runtime             │
│    └── claw-script (embedded engines for self-evolution)   │
└────────────────────────────────────────────────────────────┘
```

The core kernel crates are **not modified**. `KernelServer` is an additive layer.

---

## Protocol

Wire encoding: JSON-RPC 2.0 messages framed with the existing `claw-pal` 4-byte BE length prefix.

### Client → Server messages

```json
// Create a new agent session
{
  "jsonrpc": "2.0", "id": 1, "method": "create_session",
  "params": {
    "system_prompt": "You are Openclaw…",
    "max_turns": 20,
    "provider": "anthropic",
    "model": "claude-sonnet-4-6"
  }
}

// Send a user message
{
  "jsonrpc": "2.0", "id": 2, "method": "send_message",
  "params": { "session_id": "s-abc123", "content": "Hello!" }
}

// Return a tool result (when client-side tools are registered)
{
  "jsonrpc": "2.0", "id": 3, "method": "tool_result",
  "params": {
    "session_id": "s-abc123",
    "call_id": "call-xyz",
    "result": { "success": true, "data": "…" }
  }
}

// Destroy a session
{
  "jsonrpc": "2.0", "id": 4, "method": "destroy_session",
  "params": { "session_id": "s-abc123" }
}
```

### Server → Client messages

```json
// Session created
{ "jsonrpc": "2.0", "id": 1, "result": { "session_id": "s-abc123" } }

// Streaming token chunk
{ "jsonrpc": "2.0", "method": "chunk",
  "params": { "session_id": "s-abc123", "content": "Hi " } }

// Tool call request (client must respond with tool_result)
{ "jsonrpc": "2.0", "method": "tool_call",
  "params": {
    "session_id": "s-abc123",
    "call_id": "call-xyz",
    "tool": "web_search",
    "args": { "query": "rust programming" }
  }
}

// Turn complete
{ "jsonrpc": "2.0", "method": "finish",
  "params": {
    "session_id": "s-abc123",
    "finish_reason": "stop",
    "usage": { "prompt_tokens": 120, "completion_tokens": 45, "total_tokens": 165 }
  }
}
```

### Client SDK (Python example, ~50 lines)

```python
import socket, json, struct

class KernelClient:
    def __init__(self, socket_path="/tmp/claw-kernel.sock"):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(socket_path)
        self._id = 0

    def _send(self, method, params):
        self._id += 1
        msg = json.dumps({"jsonrpc":"2.0","id":self._id,"method":method,"params":params})
        payload = msg.encode()
        self.sock.sendall(struct.pack(">I", len(payload)) + payload)

    def _recv(self):
        length = struct.unpack(">I", self.sock.recv(4))[0]
        return json.loads(self.sock.recv(length))

    def create_session(self, system_prompt, **kwargs):
        self._send("create_session", {"system_prompt": system_prompt, **kwargs})
        return self._recv()["result"]["session_id"]

    def send_message(self, session_id, content):
        self._send("send_message", {"session_id": session_id, "content": content})
        chunks = []
        while True:
            msg = self._recv()
            if msg.get("method") == "chunk":
                chunks.append(msg["params"]["content"])
            elif msg.get("method") == "finish":
                return "".join(chunks)
```

```python
# Openclaw in Python — replaces the entire Rust reimplementation
kernel = KernelClient()
session = kernel.create_session("You are Openclaw, a helpful assistant.")
response = kernel.send_message(session, "What is Rust?")
print(response)
```

---

## Performance

IPC overhead is negligible for LLM agent workloads.

| Operation | Latency |
|-----------|---------|
| LLM API call | 500 ms – 30,000 ms |
| Tool execution | 10 ms – 5,000 ms |
| Unix socket round-trip | 0.001 ms – 0.01 ms |
| JSON serialize/deserialize | 0.01 ms – 0.1 ms |

**IPC overhead ≈ 0.001% of total response time.** The kernel's Rust performance advantage (serde speed, zero-cost async, no GC pauses, concurrent tool dispatch) is fully preserved because all computationally intensive work happens inside the kernel process, transparent to the client language.

This pattern is proven at scale: `rust-analyzer` serves VS Code via LSP socket; Redis serves every language via TCP — neither suffers from the IPC boundary.

---

## Implementation plan

### New crate: `claw-server`

```
crates/claw-server/
├── src/
│   ├── lib.rs
│   ├── server.rs        # KernelServer: accept connections, manage sessions
│   ├── session.rs       # SessionManager: AgentLoop lifecycle per connection
│   ├── protocol.rs      # JSON-RPC message types (serde)
│   └── framing.rs       # reuse claw-pal framing
└── Cargo.toml
```

`claw-server` depends on `claw-loop`, `claw-runtime`, `claw-pal`. It does **not** modify any existing crate.

### Binary: `claw-kernel-server`

```
claw-kernel-server/
└── src/main.rs   # CLI: --socket-path, --provider, --power-key …
```

### SDK repositories (separate repos, thin wrappers)

| Language | Repo |
|----------|------|
| Python | `claw-sdk-python` |
| TypeScript/Node | `claw-sdk-ts` |
| Go | `claw-sdk-go` |

Each SDK is ~100–200 lines implementing the framing + JSON-RPC client.

---

## Relationship to embedded script engines

These two features serve completely different purposes and must not be conflated:

| | Embedded scripts (`claw-script`) | IPC Daemon (`claw-server`) |
|-|----------------------------------|---------------------------|
| **Who writes code** | The agent (at runtime) | Human developers |
| **Language** | Lua / JS / Python | Any |
| **Purpose** | Self-evolution: agent generates new tools | Build full applications |
| **Execution** | Inside kernel process | Separate client process |
| **Access to bridges** | Full Rust bridge API | Protocol-defined API surface |
| **Example** | Agent writes a Lua scraper tool | Openclaw built in TypeScript |

Embedded engines remain the correct mechanism for agent self-evolution. The IPC daemon is not a replacement — it is a complementary integration point for a different audience.

---

## Consequences

### Easier

- Any language can build Openclaw-equivalent applications with ~100 lines of SDK code
- The kernel eliminates duplicate infrastructure across the entire Claw ecosystem regardless of language
- Client applications are isolated from kernel crashes / upgrades (separate processes)
- Security boundary is cleaner: client cannot directly access kernel internals

### Harder

- Two deployment artifacts instead of one (application + kernel daemon)
- Local IPC setup required (socket path, process lifecycle management)
- Cross-process debugging is harder than in-process debugging
- Protocol versioning must be managed carefully

### Not affected

- Rust users continue to use `AgentLoopBuilder` directly (zero overhead, zero protocol)
- Existing crate APIs are unchanged
- Embedded script engine behavior is unchanged

---

## Related

- [ADR-005](005-ipc-multi-agent.md) — IPC and multi-agent coordination (existing IPC layer)
- [ADR-010](010-memory-system-boundary.md) — memory system boundary
- [`claw-pal` framing](../../crates/claw-pal/src/ipc/framing.rs) — wire encoding (4-byte BE)
- [`claw-runtime` IpcRouter](../../crates/claw-runtime/src/ipc_router.rs) — existing routing infrastructure