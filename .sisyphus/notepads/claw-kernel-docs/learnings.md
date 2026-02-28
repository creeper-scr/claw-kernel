# Learnings: claw-kernel-docs

## 2026-02-28 — agent-loop-state-machine.md

### Document format
- All docs use YAML frontmatter with: title, description, status, version, last_updated, and crate/layer fields where relevant.
- Bilingual format: English section first with `<a name="english">` anchor, Chinese section second with `<a name="chinese">` anchor.
- Navigation link at top: `[English](#english) | [中文](#chinese)`
- ADR files use `---` horizontal rules between sections.

### Content patterns
- BUILD_PLAN.md Phase 4 (lines 280-358) defines the data structures but not the runtime algorithm — this gap is what the design doc fills.
- The `docs/crates/claw-loop.md` file covers the public API surface (builder pattern, stop conditions, streaming usage) but not the internal execution model.
- The `docs/design/` directory did not exist before this task — it was created implicitly by writing the file.

### Key design decisions captured
- Stop condition evaluation order is deterministic: MaxTurns → TokenBudget → NoToolCall → Custom.
- HistoryManager uses sliding window with 80% threshold for proactive summarization.
- Tool calls within one LLM response run concurrently via tokio::join_all.
- Streaming uses bounded mpsc channel (capacity 64) with backpressure.
- UserInterrupt returns Ok(AgentResult), not Err — partial results are valid.

## channel-message-protocol.md (2026-02-28)

- Created `docs/design/` directory (new, didn't exist before)
- `ChannelMessage` uses `#[serde(tag = "type")]` on `MessageContent` for self-describing JSON
- Binary fields (`Vec<u8>`) need `serde_with` base64 helper or custom serializer for JSON
- `ChannelMetadata.raw: Option<serde_json::Value>` is intentionally untyped — platform APIs change too fast for typed structs
- Telegram text limit: 4096 chars; Discord: 2000 chars — both need splitting logic in adapters
- Rate limit defaults: Telegram 30/min (0.5/sec), Discord 5/5sec (0.2/sec), Webhook 100/10sec
- Retry: 3 attempts, 1s/2s delays, ±25% jitter, max 60s cap
- `ChannelError::is_retryable()` pattern cleanly separates permanent vs transient errors
- ADR-006 three-tier pattern (Format/Transport/Provider) is a good reference for channel adapter design
- `Platform::Custom(String)` variant allows extension without upstream changes

## Community files rewrite (2026-02-28)

### CONTRIBUTING.md
- Original was 486 lines; rewrite targets <= 400 lines (achieved 321).
- Key additions over original: explicit fork-and-PR workflow, Conventional Commits convention, first-contribution guidance, Development FAQ section.
- Chinese section uses condensed prose (not full translation) to stay under line limit while keeping bilingual requirement.
- `chore/`, `refactor/`, `test/` branch prefixes added beyond the original `feat/`, `fix/`, `docs/`.
- The edit tool requires reading the last line tag before replacing the full file range.

### CHANGELOG.md
- Original was 35 lines with a vague `[Unreleased]` section.
- Rewrite uses bilingual HTML comment header (not visible in rendered output) to explain format in both languages.
- `[Unreleased]` section now lists actual design/documentation work completed in planning phase.
- `[0.1.0]` section uses `### Planned` (not `### Added`) since nothing is implemented yet.
- Exactly 60 lines achieved.

### SECURITY.md
- Key improvements: explicit "Do NOT use public GitHub Issues" warning, 90-day fix timeline (was missing), Hall of Fame section, follow-up escalation path if no 48h response.
- CVE policy clarified: CVSS >= 7.0 OR any Safe Mode sandbox escape.
- Chinese section is a full translation since SECURITY.md has no line limit constraint.
- Placeholder email `security@claw-project.dev` used throughout.
