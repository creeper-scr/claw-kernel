# claw-kernel Examples

This directory contains example applications demonstrating claw-kernel capabilities.

## Examples

### simple-agent
A basic agent with a single LLM provider and static tools.

**Demonstrates:** `AgentLoop` setup, tool registry, provider configuration.

### custom-tool
Shows how to create and register custom tools using Lua scripts.

**Demonstrates:** Lua tool scripts, tool schema definition, permission annotations.

### self-evolving-agent
An agent that generates and loads new tools at runtime.

**Demonstrates:** Tool generation via LLM, hot-reload mechanism, version management.

## Running

```bash
cd examples/simple-agent
cargo run
```
