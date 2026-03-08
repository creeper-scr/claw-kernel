---
title: "ADR-003: Dual-Mode Security"
description: "Dual-mode security design (Safe Mode and Power Mode)"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-08"
language: en
---


# ADR 003: Dual-Mode Security (Safe/Power)

**Status:** Accepted  
**Date:** 2024-01-25  
**Deciders:** claw-kernel core team, security review

---

## Context

Agents have conflicting security requirements:

1. **Default use case:** Execute LLM-generated code safely
   - Should not delete random files
   - Should not exfiltrate data
   - Should be deployable to shared environments

2. **Power use case:** Full system automation
   - Install software
   - Manage system services
   - Modify system configuration

We need a clear security model that addresses both.

---

## Decision

Implement **two explicit execution modes**:

| Aspect | Safe Mode (Default) | Power Mode (Opt-in) |
|--------|---------------------|---------------------|
| **Filesystem** | Allowlist read-only | Full access |
| **Network** | Domain/port rules | Unrestricted |
| **Subprocess** | Blocked | Allowed |
| **Self-modification** | Allowed (sandboxed) | Allowed (global) |
| **Activation** | Default | `--power-mode --power-key <key>` |

### Key Design Principles

**1. Explicit Opt-in Required**

Power Mode requires BOTH:
- `--power-mode` flag (explicit intent)
- `--power-key <key>` (authentication)

**2. No Downgrade Without Restart**

Power Mode → Safe Mode requires process restart. This prevents:
- Compromised Power Mode agent hiding evidence
- Race conditions in mode switching

**3. Kernel Immutable in Both Modes**

Rust Hard Core (Layer 0) is untouchable regardless of mode:
- No script can modify kernel code
- No script can access kernel credential storage
- No script can bypass sandbox enforcement

### Mode Switching Flow

```
┌─────────────┐      --power-mode + --power-key      ┌─────────────┐
│  Safe Mode  │  ─────────────────────────────────►  │  Power Mode │
│  (default)  │                                     │  (opt-in)   │
└─────────────┘                                     └─────────────┘
       ▲                                                    │
       │              restart or new process                 │
       └─────────────────────────────────────────────────────┘
```

---

## Consequences

### Positive

- **Clear mental model:** Users understand the trade-off
- **Safe by default:** No accidental full system access
- **Audit trail:** Mode switches are logged
- **Deployable:** Safe Mode suitable for shared/cloud environments

### Negative

- **UX friction:** Power Mode requires key management
- **Implementation complexity:** Two sandbox code paths

### Security Boundaries

**Safe Mode Guarantees (violations are bugs):**
- Scripts cannot access files outside allowlist
- Scripts cannot spawn subprocesses
- Scripts cannot make network calls outside rules
- Scripts cannot escalate to Power Mode without key
- Kernel secrets remain inaccessible

**Power Mode Guarantees:**
- Full system access BY DESIGN
- Only protection: unauthorized activation is blocked

---

## Alternatives Considered

### Alternative 1: Single Mode with Permission Prompts

**Rejected:** UX nightmare, prompts become muscle memory

### Alternative 2: Capability System (like Android)

**Rejected:** Too complex for CLI tools, overkill for our use case

### Alternative 3: Container/Docker Isolation

**Considered:** Excellent isolation, but:
- Requires Docker (not always available)
- Startup latency
- Complex volume mounting for file access

**Decision:** Use as implementation detail for sandboxing, not primary interface

---

## Implementation

### Power Key Management

**Design Decision**: Power Key is user-defined (not system-generated)

```rust
pub struct PowerKey {
    // Derived from user-provided key via Argon2
    verification_hash: [u8; 32],
}

impl PowerKey {
    pub fn verify(provided: &str) -> bool {
        let hash = argon2::hash_raw(provided.as_bytes(), SALT, PARAMS)?;
        constant_time_eq(&hash, &self.verification_hash)
    }
}
```

Key setup (user-defined):
```bash
# User sets their own power key
claw-kernel --set-power-key
Enter new power key: ********
Confirm power key: ********
Power key set successfully.
```

Key storage:
- Interactive: Prompt for key on `--power-mode`
- Config file: `~/.config/claw-kernel/power.key` (600 permissions, stores hash only)
- Environment: `CLAW_KERNEL_POWER_KEY` (not recommended for regular use)

**Security Note**: If power key is forgotten, user must reset via `--reset-power-key` (requires manual confirmation).

### Sandbox Configuration

```rust
pub struct SandboxConfig {
    pub mode: ExecutionMode,
    pub filesystem_allowlist: Vec<PathBuf>,
    pub network_rules: Vec<NetRule>,
    pub allow_subprocess: bool,
}

/// Network rule - defines allowed/denied network access
pub struct NetRule {
    pub host: String,        // Hostname or IP address
    pub port: Option<u16>,   // Port (None = all ports)
    pub allow: bool,         // true = allow, false = deny
}

impl NetRule {
    pub fn allow(host: String) -> Self { ... }
    pub fn allow_port(host: String, port: u16) -> Self { ... }
    pub fn deny(host: String) -> Self { ... }
}

/// Syscall filtering policy
pub enum SyscallPolicy {
    AllowAll,                 // No syscall restrictions
    DenyAll,                  // Block dangerous syscalls
    Allowlist(Vec<String>),   // Only allow listed syscalls
}

/// Resource limits for sandboxed processes
pub struct ResourceLimits {
    pub max_memory_bytes: Option<u64>,     // Maximum memory
    pub max_cpu_percent: Option<u8>,       // CPU limit (0-100)
    pub max_file_descriptors: Option<u32>, // Max open files
    pub max_processes: Option<u32>,        // Max processes
}

impl SandboxConfig {
    pub fn safe_default() -> Self {
        Self {
            mode: ExecutionMode::Safe,
            filesystem_allowlist: vec![
                dirs::data_dir().unwrap(),
                dirs::cache_dir().unwrap(),
            ],
            network_rules: vec![
                NetRule::allow_port("api.openai.com".to_string(), 443),
                NetRule::allow_port("api.anthropic.com".to_string(), 443),
            ],
            allow_subprocess: false,
        }
    }
    
    pub fn power_mode() -> Self {
        Self {
            mode: ExecutionMode::Power,
            filesystem_allowlist: vec![],  // No restriction
            network_rules: vec![],         // No restriction
            allow_subprocess: true,
        }
    }
}
```

---

## Security Audit Checklist

Before release:

- [ ] Safe Mode sandbox escape attempts
- [ ] Power Mode key brute force resistance
- [ ] Credential storage encryption
- [ ] Mode transition race conditions
- [ ] Audit log completeness

---

## References

- [Security Policy](../../SECURITY.md)
- [Safe Mode Guide](../guides/safe-mode.md)
- [Power Mode Guide](../guides/power-mode.md)
- [Platform Abstraction Layer](../architecture/pal.md) (sandbox implementations)

---
