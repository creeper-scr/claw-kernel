[English](#english) | [中文](#chinese)

<a name="english"></a>

# Safe Mode Guide

Safe Mode is the kernel's sandbox feature (Layer 0.5). It provides sandboxed execution suitable for running LLM-generated code safely.

> ⚠️ **Note**: This guide shows the **target API design**. The `claw-kernel` crate is not yet implemented.

---

## What is Safe Mode?

Safe Mode restricts script capabilities through sandboxing:

| Capability | Safe Mode | Power Mode |
|------------|-----------|------------|
| **File System** | Allowlisted directories, read-only by default | Full access |
| **Network** | Allowed domains/ports only | Unrestricted |
| **Subprocesses** | Blocked | Allowed |
| **System Calls** | Filtered | Unrestricted |
| **Script Hot-Loading** | Allowed (subject to sandbox limits) | Allowed (global) |

---

## Two-Layer Permission Model

Safe Mode implements a **two-layer permission model**:

### Layer 1: Sandbox Permissions (Hard Constraints)
OS-level enforcement restricts what scripts *can* do:
- Filesystem allowlist
- Network domain/port rules  
- Subprocess blocking
- System call filtering

### Layer 2: Tool Declaration (Runtime Check)
Scripts declare permissions via `@permissions` annotation:
- Provides visibility to LLM (what the tool *may* do)
- Runtime validation against sandbox configuration
- **Static error if tool declares permissions beyond sandbox scope**

### Permission Resolution
```
Effective Permission = Tool Declaration ∩ Sandbox Configuration
```

| Scenario | Tool Declaration | Sandbox Config | Result |
|----------|------------------|----------------|--------|
| ✓ Consistent | `fs.read` | `/home/user` readable | Works |
| ✓ Tool more restrictive | `fs.read` (declares only) | `/home/user` readable | Works |
| ❌ Tool exceeds sandbox | `fs.write` | Read-only | **Static error at registration** |

### Tool Registration Time Check

Permission validation happens **immediately when tool is registered** (not at call time):

```rust
// Tool permissions are checked at registration time
let mut tools = ToolRegistry::new();

// If any tool in ./tools declares permissions beyond sandbox config,
// load_from_directory fails immediately with PermissionError
tools.load_from_directory("./tools").await?;

// Once loaded, all tools are guaranteed to have valid permissions
// No runtime permission checks needed during execution
```

This ensures that permission mismatches are caught early during application startup, not during tool execution.

### Security Policy

| Layer | Responsibility |
|-------|---------------|
| **Kernel** | Sandbox isolation - restricts what scripts *can* do |
| **Application** | Permission decisions - determines what scripts *may* do |

The kernel provides the sandbox mechanism. The application decides which directories, network endpoints, and capabilities to allow.

---

## Default Allowlist

### File System

```
Linux/macOS:
  ~/.local/share/claw-kernel/      # Data directory
  ~/.cache/claw-kernel/            # Cache directory
  /tmp/                            # Temp files

Windows:
  %APPDATA%\claw-kernel\           # Data directory
  %LOCALAPPDATA%\claw-kernel\cache\ # Cache directory
  %TEMP%\                          # Temp files
```

### Network

```
Allowed domains (default):
  - api.openai.com:443
  - api.anthropic.com:443
  - api.gemini.google.com:443
  - localhost:11434 (Ollama default)
```

---

## Configuring Safe Mode

### Programmatic Configuration

```rust
use claw_kernel::pal::{SandboxConfig, ExecutionMode};
use std::path::PathBuf;

let config = SandboxConfig::safe_mode()
    // Add custom directory
    .allow_directory(PathBuf::from("/home/user/projects"))
    // Add read-write permission (default is read-only)
    .allow_directory_rw(PathBuf::from("/home/user/output"))
    // Add network endpoint
    .allow_endpoint("api.example.com", 443)
    // Build config
    .build();

let runtime = Runtime::with_sandbox(config)?;
```

### Configuration File

Create `~/.config/claw-kernel/sandbox.toml`:

```toml
[sandbox]
mode = "safe"

[[sandbox.filesystem]]
path = "/home/user/projects"
access = "read"

[[sandbox.filesystem]]
path = "/home/user/output"
access = "read-write"

[[sandbox.network]]
domain = "api.example.com"
ports = [443]

[[sandbox.network]]
domain = "internal.company.net"
ports = [80, 443]
```

---

## Platform-Specific Sandboxing

### Linux (seccomp + namespaces)

Strongest sandboxing:

```rust
// Automatically uses:
// - seccomp-bpf for syscall filtering
// - mount namespace for filesystem isolation
// - network namespace for network rules
// - pid namespace for process isolation
```

### macOS (sandbox profile)

Uses native macOS sandbox:

```rust
// Generates sandbox profile like:
// (version 1)
// (allow default)
// (deny network-outbound)
// (allow network-outbound (remote unix-socket))
// (allow file-read* (subpath "/allowed/path"))
```

### Windows (AppContainer)

Uses Windows AppContainer:

```rust
// Creates low-integrity process
// Applies capability SIDs
// Uses Job Objects for resource limits
```

---

## Testing Safe Mode

### Verify Restrictions

Create a test tool:

```lua
-- test_restrictions.lua
-- @name test_restrictions
-- @description Test sandbox restrictions
-- @permissions fs.read, net.http

function M.execute(params)
    local results = {}
    
    -- Test 1: Read allowed file
    local success = pcall(function()
        rust.fs.read("~/.local/share/claw-kernel/test.txt")
    end)
    table.insert(results, "Read allowed: " .. tostring(success))
    
    -- Test 2: Read disallowed file (should fail)
    success = pcall(function()
        rust.fs.read("/etc/passwd")
    end)
    table.insert(results, "Read disallowed: " .. tostring(not success))
    
    -- Test 3: Network to allowed domain
    success = pcall(function()
        rust.net.get("https://api.openai.com/v1/models")
    end)
    table.insert(results, "Net allowed: " .. tostring(success))
    
    -- Test 4: Network to disallowed domain (should fail)
    success = pcall(function()
        rust.net.get("https://evil.com/")
    end)
    table.insert(results, "Net disallowed: " .. tostring(not success))
    
    return {
        success = true,
        result = results
    }
end
```

### Expected Output

```
Read allowed: true
Read disallowed: true  (blocked)
Net allowed: true
Net disallowed: true   (blocked)
```

---

## Safe Mode Guarantees

The following are **security guarantees** in Safe Mode. Violations are bugs:

1. **Filesystem Isolation**
   - Cannot read files outside allowlist
   - Cannot write files outside allowlist
   - Cannot escape via symlinks

2. **Network Restrictions**
   - Cannot connect to non-allowed domains
   - Cannot connect on non-allowed ports
   - DNS requests are filtered

3. **Process Restrictions**
   - Cannot spawn subprocesses
   - Cannot execute shell commands
   - Cannot load dynamic libraries outside system paths

4. **Kernel Protection**
   - Cannot modify claw-kernel configuration
   - Cannot access kernel credential storage
   - Cannot start in Power Mode without key

---

## When Safe Mode Isn't Enough

Safe Mode intentionally restricts capabilities. If your agent needs:

- Installing system packages
- Modifying system configuration
- Accessing arbitrary files
- Running shell commands

Consider:

1. **Power Mode** — Explicit opt-in for full access
2. **Specific permissions** — Add only needed directories/endpoints
3. **Container deployment** — Run entire agent in Docker

---

## Best Practices

### 1. Start Restrictive, Relax as Needed

```rust
// Begin with minimal permissions
let config = SandboxConfig::safe_mode()
    .allow_directory_rw(dirs::data_dir().unwrap())
    .build();

// Add more as agent requires
```

### 2. Audit Tool Permissions

Review what permissions tools request:

```rust
.script_audit(|script_name, permissions| {
    println!("Script '{}' requests: {:?}", script_name, permissions);
    // Return false to block
    true
})
```

### 3. Use Read-Only Where Possible

```rust
// Prefer read-only unless write is necessary
.allow_directory(PathBuf::from("/data"))      // read-only
.allow_directory_rw(PathBuf::from("/output")) // read-write
```

### 4. Monitor Audit Logs

```bash
tail -f ~/.local/share/claw-kernel/logs/audit.log
```

---

## Troubleshooting

### "Permission denied" when reading allowed file

Check:
1. Path is exactly as allowlisted (no symlinks resolving outside)
2. Parent directories have execute permission
3. File exists and is readable

### Network requests blocked to allowed domain

Check:
1. Port is allowlisted (443 for HTTPS)
2. DNS resolution succeeds
3. No HTTPS interception breaking TLS

### Tool fails with cryptic error

Enable debug logging:

```bash
RUST_LOG=claw_pal=debug cargo run
```

---

## See Also

- [Power Mode Guide](power-mode.md) — For full system access
- [Security Policy](../../SECURITY.md) — Security model details
- [Platform-specific guides](../platform/) — OS-specific sandbox behavior

---

<a name="chinese"></a>

# 安全模式指南

安全模式是内核的沙箱功能（Layer 0.5）。它提供沙盒执行环境，适合安全地运行 LLM 生成的代码。

---

## 什么是安全模式？

安全模式通过沙箱机制限制脚本能力：

| 能力 | 安全模式 | 强力模式 |
|------|----------|----------|
| **文件系统** | 仅允许列表目录，默认只读 | 完全访问 |
| **网络** | 仅允许的域名/端口 | 无限制 |
| **子进程** | 被阻止 | 允许 |
| **系统调用** | 被过滤 | 无限制 |
| **脚本热加载** | 允许（受沙箱限制） | 允许（全局） |

---

## 两层权限模型

安全模式实现了**两层权限模型**：

### 第一层：沙箱权限（硬约束）
操作系统级强制执行限制脚本*能够*做什么：
- 文件系统允许列表
- 网络域名/端口规则
- 子进程阻止
- 系统调用过滤

### 第二层：工具声明（运行时检查）
脚本通过 `@permissions` 注解声明权限：
- 为 LLM 提供可见性（工具*可能*做什么）
- 针对沙箱配置的运行时验证
- **如果工具声明的权限超出沙箱范围，则产生静态错误**

### 权限解析
```
有效权限 = 工具声明 ∩ 沙箱配置
```

| 场景 | 工具声明 | 沙箱配置 | 结果 |
|------|----------|----------|------|
| ✓ 一致 | `fs.read` | `/home/user` 可读 | 正常工作 |
| ✓ 工具更严格 | `fs.read`（仅声明） | `/home/user` 可读 | 正常工作 |
| ❌ 工具超出沙箱 | `fs.write` | 只读 | **注册时静态错误** |

### 工具注册时检查

权限验证在**工具注册时立即进行**（不是在调用时）：

```rust
// 工具权限在注册时检查
let mut tools = ToolRegistry::new();

// 如果 ./tools 中的任何工具声明的权限超出沙箱配置，
// load_from_directory 会立即返回 PermissionError
tools.load_from_directory("./tools").await?;

// 一旦加载，所有工具都保证具有有效权限
// 执行期间不需要运行时权限检查
```

这确保了权限不匹配在应用启动时被及早发现，而不是在工具执行期间。

### 安全策略

| 层级 | 职责 |
|------|------|
| **内核** | 沙箱隔离 — 限制脚本*能够*做什么 |
| **应用** | 权限决策 — 决定允许脚本*可以*做什么 |

内核提供沙箱机制，应用决定允许哪些目录、网络端点和能力。

---

## 默认允许列表

### 文件系统

```
Linux/macOS:
  ~/.local/share/claw-kernel/      # 数据目录
  ~/.cache/claw-kernel/            # 缓存目录
  /tmp/                            # 临时文件

Windows:
  %APPDATA%\claw-kernel\           # 数据目录
  %LOCALAPPDATA%\claw-kernel\cache\ # 缓存目录
  %TEMP%\                          # 临时文件
```

### 网络

```
允许的域名（默认）：
  - api.openai.com:443
  - api.anthropic.com:443
  - api.gemini.google.com:443
  - localhost:11434 (Ollama 默认)
```

---

## 配置安全模式

### 编程配置

```rust
use claw_kernel::pal::{SandboxConfig, ExecutionMode};
use std::path::PathBuf;

let config = SandboxConfig::safe_mode()
    // 添加自定义目录
    .allow_directory(PathBuf::from("/home/user/projects"))
    // 添加读写权限（默认只读）
    .allow_directory_rw(PathBuf::from("/home/user/output"))
    // 添加网络端点
    .allow_endpoint("api.example.com", 443)
    // 构建配置
    .build();

let runtime = Runtime::with_sandbox(config)?;
```

### 配置文件

创建 `~/.config/claw-kernel/sandbox.toml`：

```toml
[sandbox]
mode = "safe"

[[sandbox.filesystem]]
path = "/home/user/projects"
access = "read"

[[sandbox.filesystem]]
path = "/home/user/output"
access = "read-write"

[[sandbox.network]]
domain = "api.example.com"
ports = [443]

[[sandbox.network]]
domain = "internal.company.net"
ports = [80, 443]
```

---

## 平台特定的沙盒

### Linux（seccomp + namespaces）

最强的沙盒：

```rust
// 自动使用：
// - seccomp-bpf 进行系统调用过滤
// - mount namespace 进行文件系统隔离
// - network namespace 进行网络规则控制
// - pid namespace 进行进程隔离
```

### macOS（sandbox profile）

使用原生 macOS 沙盒：

```rust
// 生成类似以下的沙盒配置文件：
// (version 1)
// (allow default)
// (deny network-outbound)
// (allow network-outbound (remote unix-socket))
// (allow file-read* (subpath "/allowed/path"))
```

### Windows（AppContainer）

使用 Windows AppContainer：

```rust
// 创建低完整性进程
// 应用能力 SID
// 使用 Job Objects 进行资源限制
```

---

## 测试安全模式

### 验证限制

创建一个测试工具：

```lua
-- test_restrictions.lua
-- @name test_restrictions
-- @description 测试沙盒限制
-- @permissions fs.read, net.http

function M.execute(params)
    local results = {}
    
    -- 测试 1: 读取允许的文件
    local success = pcall(function()
        rust.fs.read("~/.local/share/claw-kernel/test.txt")
    end)
    table.insert(results, "读取允许: " .. tostring(success))
    
    -- 测试 2: 读取不允许的文件（应该失败）
    success = pcall(function()
        rust.fs.read("/etc/passwd")
    end)
    table.insert(results, "读取不允许: " .. tostring(not success))
    
    -- 测试 3: 访问允许的域名
    success = pcall(function()
        rust.net.get("https://api.openai.com/v1/models")
    end)
    table.insert(results, "网络允许: " .. tostring(success))
    
    -- 测试 4: 访问不允许的域名（应该失败）
    success = pcall(function()
        rust.net.get("https://evil.com/")
    end)
    table.insert(results, "网络不允许: " .. tostring(not success))
    
    return {
        success = true,
        result = results
    }
end
```

### 预期输出

```
读取允许: true
读取不允许: true  (被阻止)
网络允许: true
网络不允许: true   (被阻止)
```

---

## 安全模式保证

以下是安全模式中的**安全保证**。违反这些是 bug：

1. **文件系统隔离**
   - 无法读取允许列表外的文件
   - 无法写入允许列表外的文件
   - 无法通过符号链接逃逸

2. **网络限制**
   - 无法连接到非允许的域名
   - 无法连接到非允许的端口
   - DNS 请求被过滤

3. **进程限制**
   - 无法生成子进程
   - 无法执行 shell 命令
   - 无法加载系统路径外的动态库

4. **内核保护**
   - 无法修改 claw-kernel 配置
   - 无法访问内核凭证存储
   - 无法以强力模式启动时没有密钥

---

## 当安全模式不够用时

安全模式有意限制能力。如果你的智能体需要：

- 安装系统包
- 修改系统配置
- 访问任意文件
- 运行 shell 命令

考虑：

1. **强力模式** — 显式选择以获得完全访问权限
2. **特定权限** — 仅添加需要的目录/端点
3. **容器部署** — 在 Docker 中运行整个智能体

---

## 最佳实践

### 1. 从严格开始，按需放宽

```rust
// 从最小权限开始
let config = SandboxConfig::safe_mode()
    .allow_directory_rw(dirs::data_dir().unwrap())
    .build();

// 根据智能体需要添加更多
```

### 2. 审计工具权限

审查工具请求的权限：

```rust
.script_audit(|script_name, permissions| {
    println!("脚本 '{}' 请求: {:?}", script_name, permissions);
    // 返回 false 以阻止
    true
})
```

### 3. 尽可能使用只读

```rust
// 除非需要写入，否则优先只读
.allow_directory(PathBuf::from("/data"))      // 只读
.allow_directory_rw(PathBuf::from("/output")) // 读写
```

### 4. 监控审计日志

```bash
tail -f ~/.local/share/claw-kernel/logs/audit.log
```

---

## 故障排除

### 读取允许的文件时出现"Permission denied"

检查：
1. 路径与允许列表完全一致（没有解析到外部的符号链接）
2. 父目录有执行权限
3. 文件存在且可读

### 访问允许域名的网络请求被阻止

检查：
1. 端口在允许列表中（HTTPS 为 443）
2. DNS 解析成功
3. 没有 HTTPS 拦截破坏 TLS

### 工具出现神秘错误

启用调试日志：

```bash
RUST_LOG=claw_pal=debug cargo run
```

---

## 另请参阅

- [强力模式指南](power-mode.md) — 获取完全系统访问权限
- [安全策略](../../SECURITY.md) — 安全模型详情
- [平台特定指南](../platform/) — 操作系统特定的沙盒行为
