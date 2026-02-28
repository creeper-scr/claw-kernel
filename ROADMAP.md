# claw-kernel Roadmap

> Project roadmap and planned features

## Project Status

claw-kernel is currently in the **design and planning stage**. The `crates/` directory is empty - implementation has not yet started.

## Implementation Phases

Based on [BUILD_PLAN.md](./BUILD_PLAN.md), development will proceed in the following phases:

### Phase 1: Platform Abstraction Layer (claw-pal)
- [ ] Define `SandboxBackend` trait
- [ ] Implement Linux sandbox (seccomp-bpf + namespaces)
- [ ] Implement macOS sandbox (sandbox(7) profile)
- [ ] Implement Windows sandbox (AppContainer + Job Objects) - **Degraded mode**
- [ ] Cross-platform IPC abstraction

### Phase 2: System Runtime (claw-runtime)
- [ ] Event bus implementation
- [ ] Process management
- [ ] IPC router
- [ ] Multi-agent orchestration foundation

### Phase 3: Core Protocols
- [ ] `claw-provider`: LLM provider traits and implementations
  - [ ] Anthropic
  - [ ] OpenAI-compatible
  - [ ] Ollama
  - [ ] Azure OpenAI
- [ ] `claw-tools`: Tool registry and hot-loading
  - [ ] `Tool` trait
  - [ ] `ToolRegistry`
  - [ ] Script-based tools (Lua)

### Phase 4: Agent Loop & Memory
- [ ] `claw-loop`: Agent loop engine
  - [ ] `AgentLoop` builder
  - [ ] Stop conditions
  - [ ] History management
- [ ] `claw-memory`: Memory backends
  - [ ] In-memory backend
  - [ ] SQLite backend (optional)

### Phase 5: Script Runtime (claw-script)
- [ ] `ScriptEngine` trait and `EngineType` enum
- [ ] Lua engine (default, independent + bridge)
- [ ] Deno/V8 engine (optional)
- [ ] PyO3 engine (optional)
- [ ] Hot-reload mechanism

### Phase 6: Channel Integrations (claw-channel)
- [ ] `Channel` trait
- [ ] Telegram integration
- [ ] Discord integration
- [ ] HTTP webhook

### Phase 7: Examples & Documentation
- [ ] simple-agent example
- [ ] custom-tool example
- [ ] self-evolving-agent example
- [ ] API documentation

### Phase 8: Meta-crate (claw-kernel)
- [ ] Re-export all crates
- [ ] Feature flags configuration
- [ ] Integration tests

## Design Decisions

See [docs/adr/](./docs/adr/) for Architecture Decision Records.

Key decisions:
- **Architecture**: 5-layer architecture (Layer 0, 0.5, 1, 2, 3)
- **Security**: Dual-mode (Safe/Power) with user-defined Power Key
- **Windows Support**: Functionality degradation + warning
- **Script Engine**: Independent + bridge architecture (low coupling)
- **Permissions**: Two-layer model (Sandbox + Tool Declaration)
- **Audit Log**: Simplified, enabled by default, time-based retention

## Milestones

| Milestone | Target | Description |
|-----------|--------|-------------|
| M1 | TBD | claw-pal implemented with cross-platform sandbox |
| M2 | TBD | Core protocols (provider, tools) working |
| M3 | TBD | Full agent loop with memory |
| M4 | TBD | Script runtime with hot-reload |
| M5 | TBD | Channel integrations complete |
| M6 | TBD | v0.1.0 release |

## Contributing

Want to help? Check [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

Priority areas:
- Windows sandbox hardening
- New LLM provider implementations
- Script engine bridge improvements
- Documentation
