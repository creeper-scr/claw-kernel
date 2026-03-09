# Security Audit — claw-kernel v1.0.0

Date: 2026-03-08
Auditor: claw-kernel core team (self-review)
Scope: claw-kernel 9+2 crates

## Summary

Overall: **CONDITIONAL PASS** — ready for v1.0.0 release with documented limitations

## Findings

### PASS: Power Key hashing

- Implementation: `claw-pal/src/security.rs`
- Uses `rust-argon2 0.8` for Argon2id hashing
- Key verification uses constant-time comparison via argon2 crate
- Power key is NOT logged (verified via grep)

### PASS: IPC input validation

- Frame size limit: 16 MiB (enforced in framing layer)
- JSON parse errors handled gracefully (no panic)
- Session IDs validated as UUID v4 format

### KNOWN: Sandbox backends are stub implementations

**Severity**: Medium
**Affects**: Linux, macOS, Windows

All three platform sandbox backends (seccomp, Seatbelt, AppContainer)
store configuration but do not enforce it in v1.0.0.
Safe Mode filesystem/network rules are NOT enforced at the OS level.

**Mitigation**: Run agents in separate processes and use OS-level controls.
**Target fix**: v1.5.0 (Linux seccomp-bpf full implementation).

### PASS: No unsafe code in public API

All `unsafe` usage is internal (framing layer) and not exposed to public API.

## Dependency Security

```
Note: cargo audit could not be executed in this environment due to
Rust version constraints (requires Rust 1.85+, current is 1.83.0).

To run cargo audit manually:
    cargo install cargo-audit
    cargo audit

Current Cargo.lock contains ~250+ dependencies.
Key security-related dependencies:
    - rust-argon2 0.8 (Power Key hashing)
    - sha2 0.10 (SHA-256 for lightweight key verification)
    - libseccomp (Linux sandbox - optional)
```

### Known Advisories

No known security advisories (audit pending cargo-audit execution in CI).

## Recommendations

1. Implement full Linux seccomp-bpf sandbox by v1.5.0
2. Add macOS Seatbelt enforcement by v1.3.0
3. Add Windows AppContainer enforcement by v1.3.0
4. Regular cargo audit in CI pipeline (TASK-12)

## Sign-off

This audit is a self-review by the core team. External audit is recommended before v2.0.0.
