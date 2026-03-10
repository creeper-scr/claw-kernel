---
title: Power Mode Guide
description: Power mode activation and security guide
status: implemented
version: "1.0.0"
last_updated: "2026-03-08"
language: en
---



# Power Mode Guide

Power Mode grants agents full system access. Use it when you need agents to perform unrestricted automation tasks.

> [Info] **Note**: This guide documents the implemented API in v1.0.0. Windows sandbox is stub-only (see KNOWN-ISSUES.md).

---

## What is Power Mode?

Power Mode removes most restrictions:

| Capability | Safe Mode | Power Mode |
|------------|-----------|------------|
| **File System** | Allowlisted directories only | Full access |
| **Network** | Allowed domains only | Unrestricted |
| **Subprocesses** | Blocked | Allowed |
| **System Calls** | Filtered | Unrestricted |
| **Extensibility** | Sandbox-constrained | Full capabilities |

**Use cases:**
- System administration automation
- Software installation and configuration
- Full system backups
- Development environment setup

---

## Enabling Power Mode

### Requirements

Power Mode requires BOTH:
1. `--power-mode` flag (explicit intent)
2. `--power-key` or configured key (authentication)

### Command Line

```bash
# Interactive (prompts for key)
my-agent --power-mode

# With key file
echo "my-secret-key" > ~/.config/claw-kernel/power.key
chmod 600 ~/.config/claw-kernel/power.key
my-agent --power-mode

# With environment variable (not recommended for production)
CLAW_KERNEL_POWER_KEY=my-secret-key my-agent --power-mode

# Inline (least secure, only for testing)
my-agent --power-mode --power-key my-secret-key
```

### Programmatic

```rust
use claw_pal::security::{PowerKeyManager, PowerKeyHash};
use claw_pal::{SandboxBackend, SandboxConfig};

// Save a power key (validates and hashes with Argon2)
PowerKeyManager::save_power_key("my-secure-key-123!")?;

// Load the stored hash
let hash = PowerKeyManager::load_stored_hash()?;

// Verify a key against the hash
if hash.verify("my-secure-key-123!") {
    // Key is valid, can enter power mode
}

// Create power mode sandbox configuration
let config = SandboxConfig::power_mode();

// For Linux:
#[cfg(target_os = "linux")]
{
    use claw_pal::LinuxSandbox;
    let sandbox = LinuxSandbox::create(config)?;
    let handle = sandbox.apply()?;  // Power mode - no restrictions applied
}
```

---

## Security Model

### What Power Mode Allows

By design, Power Mode permits:
- Reading/writing any file the user can access
- Making network requests to any endpoint
- Spawning subprocesses and shell commands
- Loading dynamic libraries

### What Power Mode Still Protects

Even in Power Mode, these remain protected:
- **Kernel code** — Cannot modify claw-kernel itself
- **Credential storage** — Cannot access kernel's secure storage
- **Other users' data** — Still subject to OS permissions

### Key Protection

The power key is used for authentication only, not encryption:

```rust
// Key is hashed with Argon2
let verification_hash = argon2::hash_raw(key, SALT, PARAMS)?;

// Constant-time comparison prevents timing attacks
constant_time_eq(&provided_hash, &stored_hash)
```

---

## Mode Selection at Startup

**Important:** Execution mode is determined at process startup and cannot be changed without restart.

```
┌─────────────┐      --power-mode + key      ┌─────────────┐
│  Safe Mode  │  ─────────────────────────► │  Power Mode │
│  (default)  │                             │  (opt-in)   │
└─────────────┘                             └─────────────┘
       ▲                                            │
       │              restart with new config       │
       └────────────────────────────────────────────┘
```

Unlike dynamic mode switching, this design:
- Prevents compromised Power Mode agents from hiding by "downgrading"
- Eliminates race conditions during mode changes
- Prevents confused deputy attacks

---

## Best Practices

### 1. Use Safe Mode by Default

Only enable Power Mode for specific tasks:

```rust
// Default: Safe Mode
let config = SandboxConfig::safe_mode().build();

// Only when needed: Power Mode
if user_explicitly_requested_power() {
    let key = prompt_for_power_key()?;
    config = SandboxConfig::power_mode()
        .with_key(key)
        .build();
}
```

### 2. Audit Power Mode Usage

Log all Power Mode activations:

```rust
if config.mode == ExecutionMode::Power {
    audit_log.record(AuditEvent::PowerModeActivated {
        timestamp: Utc::now(),
        user: current_user(),
        reason: user_provided_reason(),
    });
}
```

### 3. Short-Lived Power Sessions

Minimize time in Power Mode:

```bash
# Good: Do power work, then exit
claw-agent --power-mode --task "install-dependencies"
# Exits automatically

# Risky: Long-running power mode agent
claw-agent --power-mode --interactive  # Stays in power mode
```

### 4. Separate Power Mode Agents

Consider dedicated agents for power tasks:

```
agent-safe/          # Regular agent, Safe Mode
├── Read files
└── Answer questions

agent-power/         # Admin agent, Power Mode
├── Install software
└── Modify system
```

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| **Prompt injection** | Review tool outputs before power actions |
| **Accidental deletion** | Use `--dry-run` flag when available |
| **Network exfiltration** | Firewall rules, network monitoring |
| **Cryptomining** | Resource limits, CPU monitoring |
| **Persistence** | Regular audit of cron jobs, startup items |

---

## Power Mode Configuration

### Minimal Example

```rust
use claw_kernel::{
    provider::AnthropicProvider,
    loop_::AgentLoop,
    pal::{SandboxConfig, PowerKey},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    
    // Power Mode
    let key = PowerKey::from_env()?
        .ok_or("CLAW_KERNEL_POWER_KEY required")?;
    
    let config = SandboxConfig::power_mode()
        .with_key(key)
        .build();
    
    let runtime = Runtime::with_sandbox(config)?;
    
    // Agent can now do anything
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .runtime(runtime)
        .build();
    
    agent.run("Install nginx and configure it").await?;
    
    Ok(())
}
```

### With Resource Limits

Even in Power Mode, apply resource constraints:

```rust
let config = SandboxConfig::power_mode()
    .with_key(key)
    // Limit subprocesses
    .max_subprocesses(10)
    // Limit memory
    .max_memory_mb(2048)
    // Limit execution time
    .max_execution_time(Duration::from_hours(1))
    .build();
```

---

## Troubleshooting

### "Power key required"

You forgot the key:
```bash
# Wrong
my-agent --power-mode

# Right
my-agent --power-mode --power-key $(cat ~/.config/claw-kernel/power.key)
```

### "Cannot downgrade to Safe Mode"

By design. Restart the process:
```bash
# Kill power mode agent
pkill my-agent

# Restart in Safe Mode
my-agent
```

### Tool still restricted in Power Mode

Check if tool declares permissions:
```lua
-- This tool will still check permissions even in Power Mode
-- @permissions fs.read  <-- Required!
```

Power Mode bypasses OS-level restrictions, but tool-level permission checks may still apply unless configured otherwise.

---

## Comparison with Containers

| Approach | Isolation | Convenience | Use Case |
|----------|-----------|-------------|----------|
| **Safe Mode** | Strong | Easy | Default, untrusted code |
| **Power Mode** | None | Easy | Trusted automation |
| **Docker** | Strong | Medium | Deployment isolation |
| **VM** | Very Strong | Hard | Maximum isolation |

Power Mode + Docker:
```bash
# Run agent with power mode inside container
docker run --cap-add=ALL claw-agent --power-mode
# Power within container, container isolated from host
```

---

## See Also

- [Safe Mode Guide](safe-mode.md) — Restricted execution
- [Security Policy](../../SECURITY.md) — Complete security model

---
