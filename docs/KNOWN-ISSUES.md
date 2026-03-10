# Known Issues — claw-kernel v1.0.0

> **⚠️ Security Notice**
> 
> This document lists known security limitations. **Contributions welcome!**
> - Windows AppContainer sandbox implementation
> - Security audit of all sandbox implementations
> - Penetration testing and vulnerability research
> 
> See [SECURITY.md](../SECURITY.md) for vulnerability reporting guidelines.

## KI-001: Windows sandbox has partial isolation (Job Objects only)

**Severity**: Medium
**Affects**: Windows

The Windows sandbox backend uses **Job Objects** to enforce memory limits and
block child process spawning, but does **not** enforce filesystem or network
isolation. Linux (seccomp-bpf + Landlock LSM) and macOS (Seatbelt/`sandbox_init()`)
sandbox backends are fully implemented and enforce all restrictions.

**Status per platform:**
- ✅ Linux: Full seccomp-bpf + Landlock LSM implementation
- ✅ macOS: Full Seatbelt sandbox profile (`sandbox_init()` FFI + SBPL)
- ⚠️ Windows: **Partial** — Job Objects enforce memory limits and block child
  processes, but **filesystem and network isolation are NOT enforced**.
  AppContainer-based FS/network isolation is planned for v1.7.0.

> **⚠️ Windows Security Warning**: On Windows, agents running in "sandboxed" mode
> can still read/write arbitrary files and make unrestricted network connections.
> Do NOT rely on claw-kernel's sandbox for filesystem or network isolation on Windows.
> Use OS-level controls (ACLs, Windows Firewall) as a compensating control.

**Mitigation**: On Windows, run agents in separate processes and use OS-level
access controls (NTFS ACLs, Windows Firewall rules) to restrict filesystem and
network access.
**Target fix**: v1.7.0 (AppContainer + full Job Objects integration).

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
