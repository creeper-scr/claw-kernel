# Semver Policy

## Guarantees (v1.0+)

claw-kernel follows Semantic Versioning 2.0.0.

- **Patch (1.0.x)**: Bug fixes only. No API changes. Safe to update.
- **Minor (1.x.0)**: New features, additions. No breaking changes. Safe to update.
- **Major (x.0.0)**: Breaking changes to public API.

## What counts as "public API"

- All items accessible via `claw_kernel::prelude::*`
- All `pub` items in each sub-crate's documented API surface
- Wire protocol compatibility for claw-server (JSON-RPC message structure)
- Feature flag names

## What is NOT guaranteed

- Items marked `#[doc(hidden)]`
- Items in `#[cfg(test)]` or `#[cfg(feature="test-utils")]`
- Internal module paths (access via re-exports only)
- Behavior of `#[non_exhaustive]` enum variants (new variants are non-breaking)

## Deprecation policy

Deprecated items are marked with `#[deprecated(since="...", note="...")]`
and remain in the API for at least one minor release before removal.

## Pre-1.0 note

All 0.x.y versions are considered unstable. No compatibility guarantee applies.
