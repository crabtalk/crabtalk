# Phase 1: Core

Expand walrus-core with new traits and types, and create walrus-sqlite.

## Unit Index

| Unit | Title | Depends On |
|------|-------|------------|
| [P1-00](./units/P1-00.md) | Migrate core types to performance primitives | — |
| [P1-01](./units/P1-01.md) | Revise Memory trait | P1-00 |
| [P1-02](./units/P1-02.md) | Add MemoryEntry and RecallOptions types | P1-01 |
| [P1-03](./units/P1-03.md) | Add Channel trait and types | P1-00 |
| [P1-04](./units/P1-04.md) | Add Skill struct and SkillTier | P1-00 |
| [P1-05](./units/P1-05.md) | Add Embedder trait | P1-00 |
| [P1-06](./units/P1-06.md) | Update Agent struct (skill_tags) | P1-00 |
| [P1-07](./units/P1-07.md) | Create walrus-sqlite crate with schema | P1-01, P1-02, P1-05 |
| [P1-08](./units/P1-08.md) | Implement SqliteMemory CRUD + FTS | P1-07 |
| [P1-09](./units/P1-09.md) | Implement SqliteMemory recall + compile_relevant | P1-08 |

## Dependency Graph

```text
P1-00 ─→ P1-01 ─→ P1-02 ─→ P1-07 ─→ P1-08 ─→ P1-09
     ├─→ P1-03  (after P1-00)
     ├─→ P1-04  (after P1-00)
     ├─→ P1-05 ──────────↗
     └─→ P1-06  (after P1-00)
```

P1-00 is the foundation. After it completes, P1-01, P1-03, P1-04, P1-05, P1-06
can proceed in parallel. P1-07 through P1-09 are sequential.

## Workspace Changes

Add to `[workspace.dependencies]`:

```toml
rusqlite = { version = "0.34", features = ["bundled"] }
compact_str = { version = "0.8", features = ["serde"] }
smallvec = { version = "1", features = ["serde"] }
bytes = "1"
```

Add `crates/sqlite` to workspace members.

## Deviations

- **smallvec v1 (not v2)**: Original plan specified v2, but v2 is alpha-only.
  Using v1 which is stable and widely used.
- **Skill struct aligned with agentskills.io (P1-04)**: Original spec had
  custom fields (version, tier, tags, triggers, tools, priority). Replaced
  with agentskills.io-compliant fields: name, description, license,
  compatibility, metadata (`BTreeMap`), allowed_tools, body. Runtime concepts
  (tier, priority) moved to SkillRegistry.
- **Chat relocated to walrus-runtime**: Originally in walrus-core. Moved to
  walrus-runtime since it composes LLM messages (runtime concern, not shared
  vocabulary).

## Completion Checklist

- [x] All 10 units complete
- [x] `cargo check --workspace` passes
- [x] `cargo test --workspace` passes (55 tests)
- [x] `docs/src/design.md` updated to reflect actual implementations
- [x] No unresolved design questions remain
