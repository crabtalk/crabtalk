# 0203 - Client-Side Orchestration

- Feature Name: Client-Side Orchestration
- Start Date: 2026-08-15
- Discussion: TBD
- Crates: tui, core, crabtalk, runtime, hooks, sdk
- Updates: [0082 (Scoping)](0082-scoping.md), [0121 (Event Bus)](0121-event-bus.md), [0189 (Policy at the Edge)](0189-policy-at-the-edge.md)

## Summary

`delegate` stops being a daemon-side hook and becomes a client tool, declared in `StreamMsg.tools` like `bash`. Fan-out is then just more streams on the same socket: a conversation is keyed by `(agent, sender)`, so a distinct sender buys each sub-agent its own conversation, its own bridge listener, and its own tool set. The daemon no longer knows the tool exists — it forwards an opaque call and receives a reply. No protocol change.

Two consequences follow. Sub-agents gain the client's OS tools, which they never had. And client-provided tools now pass through the agent's `config.tools` whitelist when they are advertised, not only when they are dispatched.

## Motivation

The seam between daemon and client was already drawn, and orchestration was on the wrong side of it. The daemon owns the model loop and sessions; the client owns the environment. `bash`, `read`, `edit`, and `ask_user` execute in the client, and `hooks::os::discover_instructions` states the rule outright: the daemon does not read the user's filesystem.

Against that, look at what a delegated sub-agent actually got. `DelegateHook` called `send_to(conversation_id, &message, &sender, None, vec![])` — an empty `extra_tools`, so no client tools at all. And had it passed them, `ClientBridge::dispatch` refuses any conversation with no registered listener, and the only thing that registers one is a client's own stream. Sub-agents could think, reach memory, skills, and MCP, and nothing else. Daemon-side orchestration was orchestrating agents with no hands.

Fixing that in the daemon is not a small change. It means multiplexing sub-conversation forwards onto the parent's listener, namespacing `call_id`s across conversations, teaching the client to answer for conversations it never opened, propagating cancellation through the tree, and stopping listener teardown from killing in-flight sub-agent calls when the parent stream ends. That is a second dimension on the bridge protocol, bought to reach a capability the client already has.

In the client it is arithmetic. The bridge is already keyed per conversation with its own tool set and listener — precisely the primitive parallel orchestration needs. This is also what the boundary contract already says: the runtime owns mechanics, clients own UX, and discretionary lifecycle is composed in the client from runtime primitives. Delegation is discretionary composition.

## Design

### Delegate as a client tool

The client declares `delegate` alongside its OS tools at stream time. The daemon advertises the schema to the model, and when the model calls it, `ClientBridge` forwards it back like any other client tool. The schema and executor live entirely in the client; nothing about delegation appears in the protocol, in `crabtalk`, or in `runtime`.

```
TUI ──stream(sender="user", tools=[bash,read,edit,ask_user,delegate])──▶ daemon
     ◀───────────── ToolCallForward{name:"delegate", call_id}
     ├─ stream(agent="reviewer", sender="delegate:{call_id}:0", tools=[bash,read,edit]) ──▶
     ├─ stream(agent="tester",   sender="delegate:{call_id}:1", tools=[bash,read,edit]) ──▶
     └─ ReplyToTool(call_id, [{agent,result,error}, …]) ──────▶
```

Each sub-stream is an ordinary persisted conversation. Its own `ToolCallForward` events come back on its own stream and are answered from the same `OsHook` the parent uses, so the read-before-edit invariant holds across the whole tree. When a task finishes, the client kills its conversation, mirroring what the hook did with `rt.close`.

### Named agents only

A task targets a registered agent by name. There is no ad-hoc system prompt: `StreamMsg` carries none, and the runtime's ephemeral-agent registry was reachable only from inside the daemon. An agent that does not exist yet is created first, through `CreateAgentMsg`, as a persisted agent with a real identity — consistent with RFC 0135, where agents are the artifact users see and share. Delegation minting and destroying throwaway agents behind the user's back would pollute that namespace.

Unregistered targets fail cleanly: `get_or_create_conversation` bails, and the model receives `{"agent": "x", "error": "agent 'x' not registered"}`, which is the right nudge.

### Peer awareness follows the tool

The model learns which agents it can target from an `<agents>` block naming them. That block used to be built daemon-side: `Hooks` kept an `agent_descriptions` map, populated from `on_register_agent`, and appended the block to every agent's system prompt during `on_build_agent`.

It moves to the client, for the same reason `delegate` did. Nothing daemon-side ever read `agent_descriptions` — its only consumer was the prompt injection, in service of a tool the daemon no longer implements. Leaving it behind would have kept exactly the inversion this RFC removes, one file over.

The move also fixes a defect that the daemon-side version could not express. A delegated sub-agent is not offered `delegate`, but `on_build_agent` had no way to know that, so every sub-agent was told about siblings it had no way to reach. Composing the block in the client makes the two decisions one: the stream that declares the tool describes its targets, and the stream that withholds it says nothing. No rule to enforce, and no way to get it wrong.

Clients send the block once per conversation rather than per turn. It is a stable preamble — repeating it would waste tokens and shift the prompt prefix under caching. The TUI arms it at REPL start and re-arms on `/clear`, which kills the conversation.

One rule changed shape in the move. The daemon excluded the default agent from the peer list by name; the client filters on "has a description" alone. The scaffolded `crab` carries no description, so the outcome is unchanged today, but the rule is now a property of the agent rather than a hardcoded name — describe an agent and it becomes a target.

### Offer and gate

A sub-agent's tool set has two independent authorities, and conflating them is the mistake this design avoids.

The **client offers** what it can execute. That list is a property of the client, not of any agent, and the omissions are the interesting part: no `ask_user`, because the REPL's ask modal is a single slot that two concurrent sub-agents would corrupt; no `delegate`, because withholding it caps recursion at one level with no depth counter to maintain. A client with a queued ask UI would make a different call, by writing a different list.

The **agent config gates** what this particular agent may use. `AgentConfig.tools` is the whitelist RFC 0082 designed for exactly this case, and `Hooks::dispatch` enforces it ahead of the bridge, so it already covered client tools at dispatch. What it did not cover was advertisement: `Agent::extend_tools` appended client tools after `filtered_snapshot` had narrowed the daemon-side ones, so a scoped agent was shown `bash`, called it, and ate a rejection. `extend_tools` now applies the same whitelist, with empty-means-unrestricted semantics to match.

So scoping a sub-agent is configuring the agent. It is deliberately not a client knob — a second policy layer would duplicate the first and, being advisory rather than enforced, would disagree with it.

### What was removed

- `DelegateHook`. It could not coexist with a client tool of the same name: `SystemEnv::dispatch` consults daemon hooks before the bridge, so a daemon-side `delegate` permanently shadows a client-declared one.
- The ephemeral-agent registry (`add_ephemeral`, `remove_ephemeral`, `Runtime::ephemeral_agents`). `DelegateHook` was its only consumer. With it gone, `resolve_agent` and `has_agent` no longer need to be async.
- `DelegateTask.system_prompt` and `Delegate.background`. Background delegation returned task IDs that nothing in the tree consumed; see the note on 0121 below.

Headless clients are not left behind. Telegram, WeChat, and cron declare no tools today, so their sub-agents were already handless; they can declare `delegate` and use the same executor for the same behavior, with the fan-out living in one place instead of two.

## Updates

### 0082 - Scoping

Scoping now applies to client tools at advertisement as well as at dispatch, via `Agent::extend_tools`. The whitelist vocabulary was always written for this — `BASE_TOOLS` lists `bash`, `ask_user`, `read`, and `edit`, none of which the daemon executes.

The claim that "the `delegate` tool is always available — delegation is not gated by scope" no longer holds mechanically. `delegate` is a client tool now, so a client decides whether to offer it, and the agent whitelist filters it like any other. The intent survives: no client withholds it from a primary agent, and delegation remains ungated between registered agents.

"Delegate CWD isolation" described a `cwd` field on `DelegateTask` that was never implemented; `StreamMsg` reserves tag 5 and the name `cwd` from its removal. Sub-agents share the client's working directory. The `edit` tool's unique-match requirement remains the concurrency guard, as that section already noted.

### 0189 - Policy at the Edge

0189 removed `Hook::on_before_run` and relocated the peer-agents `<agents>` block into `on_build_agent`, so it landed in the system prompt at agent-build time instead of as a per-turn injection. That was the right move within the daemon; this RFC finishes it by taking the block out of the daemon entirely.

The principle is 0189's own: mechanism belongs in the daemon, policy belongs at the edge. Which agents a client will route to, and whether it offers routing at all, is policy. `Hooks::agent_descriptions` and the block it fed are removed.

### 0121 - Event Bus

`Delegate.background` is removed. The field returned `{"task_id": "delegate:N"}` to the model, and no consumer for those IDs was ever built — the model could not await them and no client surfaced them. Client-side fan-out is synchronous by construction: the executor holds the parent's tool call open until every task settles.

This does not remove the underlying need. Work that must outlive the client process is real, and it belongs to the daemon — but as a jobs API with addressable handles, not a boolean on a tool. The event bus remains the right substrate for delivering its results out-of-band.

## Alternatives

**Multiplex sub-conversation forwards onto the parent listener.** Keep delegation in the daemon and extend the bridge so a sub-agent's tool calls ride the parent's stream. Rejected in the Motivation: it is a protocol expansion whose payoff is a capability the client already has, and it puts cancellation and permission prompts on the side of the wire that has neither the user nor the filesystem.

**Make the sub-agent tool set client-configurable.** Rejected because per-agent capability already has an enforced home in `AgentConfig.tools`. Two sources of truth for "what can this agent do" is how an agent ends up scoped in config and unscoped in practice.

**Keep a daemon-side delegate for headless clients.** Rejected: it cannot coexist with the client tool under one name, and headless clients are served by the same executor without it.

## Unresolved Questions

- `ClientBridge::FORWARD_TIMEOUT` is 300s and now bounds an entire fan-out rather than a single `bash`. It is only a backstop for a connected-but-silent client, since `unregister_listener` already fails pending calls when a stream drops — so raising it is low-risk, but the right ceiling has not been measured.
- Should a sub-agent be able to delegate? Depth is capped at one by withholding the tool, which costs nothing and is trivially reversible, but a real nested use case would want a depth counter rather than an omission.
