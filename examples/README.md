# claw-kernel Examples

This directory contains example applications demonstrating claw-kernel capabilities.

> ⚠️ **Note**: These examples are **design targets** - the `claw-kernel` crate is not yet implemented.

## Examples

### 1. simple-agent
A basic agent with single LLM provider and static tools.

**Demonstrates**:
- Basic `AgentLoop` setup
- Tool registry
- Provider configuration

Status: 🚧 Skeleton only

### 2. custom-tool
Shows how to create and register custom tools.

**Demonstrates**:
- Lua tool script
- Tool schema definition
- Permission annotations

Status: 🚧 Skeleton only

### 3. self-evolving-agent
An agent that can generate and load new tools at runtime.

**Demonstrates**:
- Tool generation via LLM
- Hot-reload mechanism
- Version management

Status: 🚧 Skeleton only

## Running Examples

Once `claw-kernel` is implemented:

```bash
cd examples/simple-agent
cargo run
```
