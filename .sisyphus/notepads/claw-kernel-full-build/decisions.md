# Architecture Decisions

## 2026-02-28 Session Start

### Decided in Planning Phase
- Only 3 LLM providers: Anthropic, OpenAI, Ollama (NOT all 9 listed in README)
- No SQLite history backend (in-memory only)
- claw-channel and claw-memory are placeholder crates only
- Windows sandbox is skeleton (stub), NOT full AppContainer implementation
- Lua is default engine; V8 and Python are feature-gated opt-ins
- Power Key: minimum 12 chars, Argon2 hash storage
- EventBus capacity: 1024 (broadcast channel)
- IPC frame format: 4-byte big-endian length prefix + payload
- Hot-reload debounce: 50ms
- IPC pattern: single reader thread + channel dispatch (NOT split socket)

### Crate Build Order
Wave 0: scaffold → Wave 1: claw-pal traits → Wave 2: claw-pal impl → Wave 3: claw-runtime + Layer 2 traits → Wave 4: Layer 2 impl → Wave 5: Layer 3 + meta
