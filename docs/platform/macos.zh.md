---
title: macOS 平台指南
description: macOS platform guide (sandbox profile)
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](macos.md)

# macOS 平台指南

macOS 通过原生 `sandbox(7)` 系统提供良好的沙箱功能。

---

## 系统要求

- macOS 10.15+（推荐 11.0+）
- Xcode 命令行工具
- Rust 工具链（稳定版）

---

## 安装

```bash
# 安装 Xcode 命令行工具
xcode-select --install

# 验证
clang --version
```

---

## 沙箱实现

macOS 使用**沙箱配置文件**：

```rust
// 生成的配置文件示例
(version 1)
(allow default)
(deny network-outbound)
(allow network-outbound 
    (remote unix-socket)
    (remote ip "api.openai.com:443"))
(allow file-read* 
    (subpath "/Users/user/Library/Application Support/claw-kernel"))
(allow file-write*
    (subpath "/Users/user/Library/Caches/claw-kernel"))
```

### 局限性

与 Linux 相比：
- 没有等同于 seccomp 的系统调用过滤功能
- 网络过滤更加受限
- 文件系统规则仅基于路径

---

## 代码签名

要进行完整的沙箱测试，需要为二进制文件签名：

```bash
# 生成自签名证书（钥匙串访问）
# 证书助理 → 创建证书...
# 名称："claw-kernel-dev"，类型：代码签名

# 为二进制文件签名
codesign -s "claw-kernel-dev" --force target/debug/my-agent

# 验证
codesign -dvv target/debug/my-agent
```

---

## 配置

### 配置目录

```
~/Library/Application Support/claw-kernel/   # 数据
~/Library/Caches/claw-kernel/                # 缓存
```

### 示例

```rust
use claw_kernel::pal::dirs;

let data_dir = dirs::data_dir();
// /Users/<user>/Library/Application Support/claw-kernel/
```

---

## 测试

```bash
# 运行测试
cargo test --workspace

# 带沙箱测试（需要已签名的二进制文件）
codesign -s "claw-kernel-dev" target/debug/deps/*
cargo test --features sandbox-tests
```

---

## 故障排除

### "sandbox_init failed"（沙箱初始化失败）

代码签名问题：

```bash
# 检查签名
codesign -dvv target/debug/my-agent

# 重新签名
codesign -s "claw-kernel-dev" --force target/debug/my-agent
```

### SIP 干扰

系统完整性保护可能会阻止某些操作：

```bash
# 检查 SIP 状态
csrutil status

# 在 Apple Silicon 上不容易禁用 SIP
# 请改用权限文件（entitlements）
```

### Gatekeeper

如果二进制文件被隔离：

```bash
# 移除隔离属性
xattr -d com.apple.quarantine target/debug/my-agent
```

---

## 性能

| 指标 | 数值 |
|-----|------|
| 沙箱开销 | ~1-2毫秒 |
| IPC 延迟 | ~15微秒（UDS） |
| 上下文切换 | 良好（原生） |

由于沙箱配置文件编译，比 Linux 稍慢。

---

## 公证（分发）

分发您的代理程序：

```bash
# 使用开发者 ID 签名
codesign -s "Developer ID Application: Your Name" \
    --options runtime \
    --entitlements entitlements.plist \
    target/release/my-agent

# 创建 DMG
# ... 

# 公证
xcrun notarytool submit my-agent.dmg --wait
```

---

## 另请参阅

- [PAL 架构](../architecture/pal.md)
- [Linux 指南](linux.md)
- [Windows 指南](windows.md)
