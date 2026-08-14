# 0121 - Event Bus

- Feature Name: Unified Event Bus
- Start Date: 2026-04-04
- Discussion: [#121](https://github.com/crabtalk/crabtalk/issues/121)
- Crates: daemon, core, runtime
- Updates: [0080 (Cron)](0080-cron.md)

> **Updated by [0203 (Client-Side Orchestration)](0203-client-side-orchestration.md) (2026-08-15).** "Non-blocking delegation" is removed with the daemon-side `delegate` hook: the `background` field returned task IDs nothing consumed. Client-side fan-out is synchronous. Detached work that outlives the client is still unsolved and belongs to a daemon jobs API.

## Summary

A daemon-level event bus that routes named events to target agents via
exact-match subscriptions. Agent completion is the first built-in event source.
The bus also enables non-blocking delegation and ad-hoc worker agents.

## Motivation

The daemon can trigger agents on a schedule (cron) and run agents on user
request (protocol). But there's no way for one agent's completion to trigger
another agent. The Signal pipeline (crabtalk/app#59) needs exactly this:

```
RSS fetch → Scout classifies → Crab enriches → client notification
```

Each stage produces a result that the next stage consumes. Without an event
system, this requires the client to orchestrate the chain — polling, waiting,
re-sending. The daemon should own this.

Separately, `delegate` blocks the parent agent until all tasks complete. For
background research or parallel work, this is a limitation. If the daemon can
route agent completion events, non-blocking delegation falls out for free.

## Design

### Event bus

An in-memory subscription table that matches events by exact source string and
fires target agents with the event payload as message content.

```toml
# events.toml
[[subscription]]
id = 1
source = "agent:scout:done"
target_agent = "crab"
once = false
```

Follows the CronStore pattern: HashMap-backed, TOML-persisted, auto-incrementing
IDs, atomic writes (tmp + rename). Survives runtime reloads.

### Event sources

Events are namespaced strings. Two source types exist today:

| Source | Example | Emitter |
|--------|---------|---------|
| Agent completion | `agent:scout:done` | Daemon, via `on_agent_event` hook |
| External | `rss:fetch`, `signal:classified` | Client or adapter, via `PublishEvent` |

Agent completion events are emitted automatically when a conversation stream
ends. The payload is the agent's final text response.

External events are published via the `PublishEvent` protocol message — any
client, adapter, or webhook handler can fire events into the bus.

### Routing

```
Event arrives (via DaemonEvent::PublishEvent)
  → event loop calls EventBus::publish() inline (no spawn)
  → exact match source against subscription table
  → for each match: fire target agent via SendMsg (fire-and-forget)
  → if once: remove subscription, persist
```

Events always start new work. There is no injection into active conversations —
that's a separate concern (#117).

Fired agents receive the payload as message content with sender
`"event:{source}"`. This follows the established convention
(`"delegate:{id}"`, `"cron"`) for non-user senders.

### Protocol

Four new operations on the `Server` trait:

```protobuf
message SubscribeEventMsg {
  string source = 1;
  string target_agent = 2;
  bool once = 3;
}

message UnsubscribeEventMsg { uint64 id = 1; }
message ListSubscriptionsMsg {}
message PublishEventMsg { string source = 1; string payload = 2; }
```

Responses: `SubscriptionInfo` for subscribe, `Pong` for unsubscribe/publish,
`SubscriptionList` for list.

### DaemonEvent::PublishEvent

All publish paths route through a single `DaemonEvent::PublishEvent` variant
in the central event loop. This avoids lock-ordering issues — the event bus
mutex is only acquired inside the sequential event loop, never from the
protocol handler or hook callbacks directly.

```rust
DaemonEvent::PublishEvent { source, payload } => {
    self.events.lock().await.publish(&source, &payload);
}
```

### Non-blocking delegation

The `delegate` tool gains a `background: bool` field. When true:

1. Tasks are spawned via the existing `spawn_agent_task` mechanism
2. `dispatch_delegate` returns immediately with task IDs
3. The parent agent continues working
4. When each task completes, the daemon emits `agent:{name}:done`
5. Event bus routes the completion to any matching subscriptions

No new mechanism — just the existing spawn infrastructure plus the event bus.

### Worker pseudo-agent

A built-in `worker` agent registered at daemon startup alongside `crab`.
Always available as a delegate target without pre-configuration:

- Inherits the system agent's thinking setting
- Gets the full tool registry (no explicit filter)
- Ephemeral — sessions are killed after task completion (existing behavior)
- Always a valid delegate target (delegation is not scoped)

This eliminates the friction of configuring named agents for ad-hoc tasks like
"read these files and summarize" or "search for X in the codebase."

## What this is NOT

- **Not a message broker.** No durability, no exactly-once delivery, no dead
  letter queues. Fire-and-forget with best-effort delivery.
- **Not an orchestration DAG.** No conditional routing, no fan-out/fan-in.
  Agents subscribe to events — that's it.
- **Not a replacement for `delegate`.** Delegation is synchronous and returns
  results inline. Events are asynchronous and deliver results out-of-band.
  `background: true` bridges the two.

## Updates

### 0080 - Cron

The cron system continues to work as-is. Cron entries fire skills via the
daemon event channel — this is unchanged. A future iteration may refactor cron
as an event source adapter, emitting `cron:{id}:fired` events into the bus, but
this is not in scope. The event bus is additive, not a cron replacement.

## Alternatives

**Agent completion triggers (no bus).** A simpler design where completion of
agent X directly triggers agent Y, without a general subscription mechanism.
Rejected because the Signal pipeline needs external events (RSS fetch results)
alongside agent completions — a bus handles both uniformly.

**Glob matching on source patterns.** The RFC originally proposed wildcard
subscriptions like `"agent:*:done"`. Rejected for v1 — exact match covers all
current use cases. Glob matching can be added when a real consumer needs it.

**Template interpolation.** The RFC originally proposed `{{payload}}`
interpolation in a `prompt_template` field. Rejected — agents are the template
engine. The payload goes in as-is; the agent's instructions handle
interpretation.

## Unresolved Questions

- Should there be a max subscription count?
- Should the bus detect infinite loops (agent A triggers B triggers A)?
  Currently fire-and-forget prevents stack overflow but allows unbounded
  chains of spawned tasks.
