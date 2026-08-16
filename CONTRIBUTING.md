# Contributing

Crabtalk is a workspace of crates and apps. The `crabtalk` library is the
product — everything else either powers it or connects to it.

## Layering

```
Layer 0 ─ Foundation
  └─ core (wcore)        Agent, Session, Runtime, Protocol, Hook, ToolRegistry

Layer 1 ─ Backends (independent of each other)
  ├─ model               ProviderRegistry (OpenAI, Anthropic, Google, Bedrock, Azure)
  └─ transport            UDS + TCP socket layers, shared Transport enum

Layer 2 ─ Engine
  └─ runtime              Env, tool dispatch, MCP, skills, memory

Layer 3 ─ Library
  ├─ storage              Storage backends (FsStorage); config scaffolding
  └─ crabtalk             Hooks, protocol, composite hook, builder (CrabTalk)

Layer 4 ─ Adapters
  ├─ crabtalkd            Daemon CLI: run, setup, reload, events
  ├─ crabup               Package manager for the ecosystem
  ├─ client               Connection, typed RPC sugars, stream adapters
  ├─ cli                  Daemon management commands
  └─ harness/             the harnesses (berm guests, RV64 ELFs)
```

Beside all of it sits `berm/` — the sandbox harnesses run in, plus its SDK
and its macro. It depends on no crate of ours, because crabtalk is one thing
that embeds it and deliberately not the only one; `berm/` and `harness/` leave
together when it does. See [RFC 0205](docs/src/rfcs/0205-berm.md).

## Where does my feature go?

| Question | Crate |
|----------|-------|
| Does it define agent behavior, protocol types, or tool schemas? | core |
| Does it add or configure an LLM provider? | model |
| Does it add a wire transport? | transport |
| Does it add a tool the agent can call, a skill, or memory? | runtime |
| Does it need network I/O, scheduling, or process lifecycle? | crabtalk (system) |
| Does it adapt a platform or parse bot commands? | client |
| Does it add a daemon admin command (over the socket)? | crabtalkd |
| Does it install or update a crabtalk binary? | crabup |
| Does it add a management subcommand to the `crabtalk` binary? | cli |
| **If none of these fit, challenge whether the feature should exist.** | |

## Boundary Contracts

- **Runtime** — never initiates I/O. It only responds. No sockets, timers, or listeners.
- **Runtime owns mechanics, clients own UX.** The runtime exposes session primitives (`new_session`, `append_message`, `list_sessions`, `list_messages`, `get_session_meta`, `search_sessions`) and runs auto-compaction only as a context-window safety net. Discretionary lifecycle — `/clear`, `/new`, `/compact`, session selection, archival browsing, saved searches — is composed in the client from those primitives. See [RFC 0185](docs/src/rfcs/0185-session-search.md).
- **Crabtalk (library)** — never interprets tool semantics. It only routes. Cron and config are system concerns (process-lifetime, not session-lifetime).
- **Client** — no dependency on runtime or model. Adapter-centric, not agent-centric.
- **Core** — defines traits and types only. If a core change pulls in runtime or daemon deps, the abstraction is wrong.

`Env` is the seam between the library and the runtime engine. The library
constructs the runtime, feeds it messages, and receives tool calls back
through the event channel.

## Data Flow

```
Client (CLI/ACP/Desktop) → UDS/TCP or in-process → CrabTalk
  → Agent.step(): config + history + tools → Model.send()/stream()
  → Tool calls dispatched via ToolDispatcher → Env.dispatch_tool()
```

## Key Types

- `Agent<P: Provider>` — immutable definition + execution (step/run/run_stream)
- `Session` — conversation history container
- `Runtime<C: Config>` — agents + sessions + tool dispatch
- `Env` — engine environment: skills, MCP, memory, tool routing
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
