# Superseded RFCs

RFCs that have been replaced by newer designs. Kept for historical reference.

| RFC | Title | Superseded by |
|-----|-------|---------------|
| [0000](0000-compaction.md) | Compaction | [0189 - Policy at the Edge](0189-policy-at-the-edge.md) |
| [0038](0038-memory.md) | Memory | [0150 - Memory Store](0150-memory-store.md) |
| [0064](0064-session.md) | Session | [0135 - Agent-First Protocol](0135-agent-first.md) |
| [0078](0078-compact-session.md) | Compact Session | [0135 - Agent-First Protocol](0135-agent-first.md) |
| [0080](0080-cron.md) | Cron | [0205 - Berm](0205-berm.md) |
| [0036](0036-skill.md) | Skill Loading | [0205 - Berm](0205-berm.md) |
| [0043](0043-component.md) | Component System | [0205 - Berm](0205-berm.md) |
| [0171](0171-topic-switching.md) | Topic Switching | [0185 - Session Search and Storage Primitives](0185-session-search.md) |
| [0150](0150-memory-store.md) | Memory Store | [0207 - Store](0207-store.md) |
| [0185](0185-session-search.md) | Session Search and Storage Primitives | [0207 - Store](0207-store.md) |
| [0075](0075-hook.md) | Hook | [0207 - Store](0207-store.md) |
| [0203](0203-client-side-orchestration.md) | Client-Side Orchestration | [0205 - Berm](0205-berm.md) |

## Reversed without a replacement

A decision can stop holding without another RFC arriving to say so. These are
the ones a reader would otherwise take as current.

| RFC | What no longer holds |
|-----|----------------------|
| [0205](0205-berm.md) | "`ask_user` stays a forwarded client tool, and so does `delegate` until it becomes a harness of its own." Client-side tool forwarding was removed whole on 2026-08-18: `SendMsg.tools`, `StreamMsg.tools`, `ToolCallForwardEvent` and `ReplyToTool` are reserved field numbers, and there is no bridge behind them. A tool runs where the runtime does. |
