# Changelog

## [0.1.0] — 2026-03-10

### Added
- `KernelClient` — synchronous client with full namespace API
- `AsyncKernelClient` — asyncio client with async generators for streaming
- `SessionNamespace` — `create`, `send`, `send_collect`, `destroy`, `tool_result`
- `AgentNamespace` — `spawn`, `kill`, `steer`, `list`, `announce`, `discover`
- `ToolNamespace` — `register`, `unregister`, `list`, `watch_dir`, `reload`
- `ChannelNamespace` — 11 methods including `inbound`, `broadcast`, routing
- `TriggerNamespace` — `add_cron`, `add_webhook`, `add_event`, `remove`, `list`
- `ProviderNamespace` — `register`, `list`
- `ScheduleNamespace` — `create`, `cancel`, `list`
- `SkillNamespace` — `load_dir`, `list`, `get_full`
- `AuditNamespace` — `list`
- `errors.py` — typed exception hierarchy
- `models.py` — dataclass models (zero dependencies)
- `_auth.py` — cross-platform path resolution + daemon auto-start
- `_transport.py` — sync framing with `bytearray` buffer
- `async_transport.py` — asyncio framing layer
- `_rpc.py` — thread-safe JSON-RPC 2.0 with per-session notification queues
- Backward-compatible deprecated methods on `KernelClient`
- Test suite: transport, models, RPC, async client
- Six example scripts
