# Phase 2: Runtime

Integrate memory, skills, and MCP into the Runtime, implement the Telegram
channel adapter, simplify runtime interfaces, and add examples.

## Unit Index

| Unit | Title | Depends On |
|------|-------|------------|
| [P2-01](./units/P2-01.md) | Add SkillRegistry to walrus-runtime | Phase 1 (P1-04) |
| [P2-02](./units/P2-02.md) | Add MCP bridge to walrus-runtime | — |
| [P2-03](./units/P2-03.md) | Integrate memory into Runtime (`Runtime<M: Memory>`, memory flush before compaction) | Phase 1 (P1-01, P1-02) |
| [P2-04](./units/P2-04.md) | Integrate skills into Runtime | P2-01 |
| [P2-05](./units/P2-05.md) | Create walrus-telegram crate | Phase 1 (P1-03) |
| [P2-06](./units/P2-06.md) | Implement Telegram Channel (connect + send) | P2-05 |
| [P2-07](./units/P2-07.md) | Team delegation (worker agents call LLM) | P2-03, P2-04 |
| [P2-08](./units/P2-08.md) | Hook trait, compaction, ergonomic API | P2-07 |
| [P2-09](./units/P2-09.md) | Simplify Runtime API (remove Chat, unify resolve) | P2-08 |
| [P2-10](./units/P2-10.md) | Runtime examples | P2-09 |
| [P2-11](./units/P2-11.md) | Wire hybrid BM25 + vector recall in walrus-sqlite | P2-09 |

## Dependency Graph

```text
P1-04 ─→ P2-01 ─→ P2-04 ─→ P2-07 ─→ P2-08 ─→ P2-09 ─→ P2-10
                          P2-02 (independent)      └──→ P2-11
P1-01, P1-02 ─→ P2-03 ─→ P2-07
P1-03 ─→ P2-05 ─→ P2-06
```

P2-02, P2-03, and P2-05 are independent of each other.
P2-10 and P2-11 are independent of each other (both depend on P2-09).

## Workspace Changes

Add to `[workspace.dependencies]`:

```toml
rmcp = { version = "0.16", features = ["client", "transport-child-process"] }
serde_yaml = "0.9"
toml = "0.8"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
dotenvy = "0.15"
```

Add `crates/telegram` to workspace members (auto-included via `crates/*` glob).

## Deviations

- **rmcp features**: Added `client` and `transport-child-process` features
  (spec said "0.16+" but didn't specify features).
- **P2-05 + P2-06 combined**: Implemented TelegramChannel skeleton and full
  connect/send in a single pass since they were sequential.
- **tokio as runtime dep**: Moved tokio from dev-dependencies to dependencies
  in walrus-runtime since McpBridge needs `tokio::process::Command` and
  `tokio::sync::Mutex` at runtime.
- **P2-09 stream_to ownership**: `stream_to()` destructures Session into raw
  `Vec<Message>` and `compaction_count` to avoid move-in-loop issues with
  `async_stream::try_stream!`. Session is reconstructed on return.

## Completion Checklist

- [x] P2-01 through P2-08 complete
- [x] `cargo check --workspace` and `cargo clippy --workspace` pass
- [x] `cargo test --workspace` passes (97 tests)
- [x] Runtime with memory/skills injects into system prompts correctly
- [x] Runtime without memory/skills behaves identically to before
- [x] Team delegation uses real LLM send loops (no stubs)
- [x] Hook trait replaces old Compactor, automatic compaction wired in
- [x] Re-exports and prelude module available
- [x] `docs/src/design.md` updated
- [x] P2-09: Chat removed, `send_to`/`stream_to` primary API, resolve unified
- [x] P2-10: Runtime examples compile and run
- [x] P2-11: Hybrid BM25 + vector recall wired in walrus-sqlite
- [x] All 11 units complete
