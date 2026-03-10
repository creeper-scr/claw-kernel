# claw-kernel Examples

This directory contains Rust example applications demonstrating claw-kernel capabilities.

## Examples

### simple-agent
A basic agent with a single LLM provider and static tools.

**Demonstrates:** `AgentLoop` setup, tool registry, provider configuration.

### custom-tool
Shows how to create and register custom tools using Lua scripts.

**Demonstrates:** Lua tool scripts, tool schema definition, permission annotations.

### memory-agent
An agent that uses claw-memory for persistent context across sessions.

**Demonstrates:** `SecureMemoryStore`, ngram embeddings, memory-augmented conversation.

### v8-scripts
An agent that runs TypeScript tool scripts via the V8 engine.

**Demonstrates:** `engine-v8` feature, async TypeScript tools, V8 host bridges.

## Running

```bash
cd examples/simple-agent
cargo run
```

## Python SDK

Python examples are located in [`sdk/python/examples/`](../sdk/python/examples/).
See [`sdk/python/README.md`](../sdk/python/README.md) for setup and usage instructions.
