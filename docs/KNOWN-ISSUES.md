# Known Issues — claw-kernel v1.0.0

## KI-001: Sandbox backends are stub implementations

**Severity**: Medium
**Affects**: Linux, macOS, Windows

All three platform sandbox backends (seccomp, Seatbelt, AppContainer)
store configuration but do not enforce it in v1.0.0.
Safe Mode filesystem/network rules are NOT enforced at the OS level.

**Mitigation**: Run agents in separate processes and use OS-level controls.
**Target fix**: v1.5.0 (Linux seccomp-bpf full implementation).

## KI-002: Windows claw-script not tested in CI

**Severity**: Low
**Affects**: Windows users of claw-script (Lua engine)

mlua Windows compatibility is not verified in CI.
The Lua engine may work on Windows but is untested.

**Target fix**: v1.3.0 (enhanced scripting release).

## KI-003: EventBus LagStrategy::Skip/Warn in try_recv

**Severity**: Low
**Affects**: Users of `EventReceiver::try_recv` and `FilteredReceiver::try_recv`
           with `LagStrategy::Skip` or `LagStrategy::Warn`

Non-blocking `try_recv` with Skip/Warn still returns `Err(Lagged)` instead of
skipping. Only the async `recv()` method correctly handles Skip/Warn.

**Workaround**: Use async `recv()` when lag handling is important.
**Target fix**: v1.1.0

## KI-004: KernelServer does not support client-side tool registration via protocol

**Severity**: Low
**Affects**: claw-server users wanting to register tools from non-Rust clients

In v1.0.0, `claw-server` supports client-side tool_result callbacks but does
not support registering tool schemas from the client. Tool definitions must be
registered server-side or the agent must not use tools.

**Target fix**: v1.1.0 (add `register_tool` method to JSON-RPC protocol).
