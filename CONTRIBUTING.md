# Contributing

Crabtalk is a workspace of crates and apps. The daemon is the product — everything
else either powers it or connects to it.

## Layering

```
Layer 0 ─ Foundation
  └─ core (wcore)        Agent, Session, Runtime, Protocol, Hook, ToolRegistry

Layer 1 ─ Backends (independent of each other)
  ├─ model               ProviderRegistry (OpenAI, Anthropic, Google, Bedrock, Azure)
  ├─ transport            UDS + TCP socket layers, shared Transport enum
  └─ command              Service management (systemd/launchd), proc macro codegen

Layer 2 ─ Engine
  └─ runtime              Env, tool dispatch, MCP, skills, memory

Layer 3 ─ Server
  └─ crabtalk             Daemon core: event loop, transport setup, cron, config, hot reload

Layer 4 ─ Adapters
  ├─ crabtalkd            Daemon CLI: start/stop/foreground, admin commands
  ├─ sdk                  NodeClient, message types for platform adapters
  ├─ tui                  REPL, config TUI (optional daemon feature for all-in-one)
  ├─ apps/                telegram, wechat (gateway clients)
  └─ plugins/             outlook, search (official plugins)
```

## Where does my feature go?

| Question | Crate |
|----------|-------|
| Does it define agent behavior, protocol types, or tool schemas? | core |
| Does it add or configure an LLM provider? | model |
| Does it add a wire transport? | transport |
| Does it add a tool the agent can call, a skill, or memory? | runtime |
| Does it need network I/O, scheduling, or process lifecycle? | crabtalk |
| Does it adapt a platform or parse bot commands? | sdk |
| Does it add a daemon admin command? | crabtalkd |
| Does it add a TUI feature or interactive UI? | tui |
| **If none of these fit, challenge whether the feature should exist.** | |

## Boundary Contracts

- **Runtime** — never initiates I/O. It only responds. No sockets, timers, or listeners.
- **Crabtalk (daemon core)** — never interprets tool semantics. It only routes. Cron and config are daemon concerns (process-lifetime, not session-lifetime).
- **SDK** — no dependency on runtime or model. Adapter-centric, not agent-centric.
- **Core** — defines traits and types only. If a core change pulls in runtime or daemon deps, the abstraction is wrong.

`Host` is the seam between daemon and runtime. The daemon constructs the
runtime, feeds it messages, and receives tool calls back through the event
channel.

## Data Flow

```
Client (TUI/Telegram/etc) → UDS/TCP → Daemon event loop
  → Agent.step(): config + history + tools → Model.send()/stream()
  → Tool calls dispatched via ToolDispatcher → Env.dispatch_tool()
```

## Key Types

- `Agent<P: Provider>` — immutable definition + execution (step/run/run_stream)
- `Session` — conversation history container
- `Runtime<C: Config>` — agents + sessions + tool dispatch
- `Env<H, S>` — engine environment: skills, MCP, memory, tool routing
- `Host` — trait for server-specific tools (ask_user, delegate, session CWD)
- `Storage` — wcore trait; pluggable KV backend reached through `Config::Storage`
- `ToolDispatcher` — wcore trait the agent calls to execute a tool
- Protocol — `ClientMessage` / `ServerMessage` (protobuf)

## External Dependencies

LLM provider implementations (auth, request formatting, streaming) live in
[`crabtalk/crabllm`](https://github.com/crabtalk/crabllm). The `model` crate
wraps `crabllm-provider` — changes to provider internals should be contributed
upstream.

## Pull Requests

- One logical change per PR. Don't mix features, refactors, and dependency changes.
- Don't vendor dependencies. If you need to patch an upstream crate, PR the fix upstream.
- Break work into reviewable commits — each commit should be one coherent change.
- Keep commits focused — each commit should have a single reason to exist.
  Mechanical changes (lockfile updates, renames, `cargo fmt`) can be large.
- PR titles use conventional commits: `type(scope): description`.

## Design

Design decisions and their rationale are documented as RFCs in the
[development book](https://crabtalk.github.io/crabtalk/)
([source](docs/src/SUMMARY.md)). Read the ones relevant to the crate you're
touching — they explain the why, not just the what.

An RFC is needed when a change defines a public contract, protocol, or
interface that external builders would implement against. Internal refactors,
bug fixes, and enhancements don't need one.
