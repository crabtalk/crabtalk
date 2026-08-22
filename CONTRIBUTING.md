# Contributing

Crabtalk is a workspace of libraries, a framework, and the products built on
them. The architecture — what a harness is, what a capability is, and which
one your feature wants — is [Architecture](docs/src/arch.md). This file is the
map: which directory the code goes in once you know.

## Layering

```
lib/                    Standalone libraries. They know nothing about the
  ├─ crabdb             runtime and would make sense without it.
  ├─ mcp
  ├─ embed
  ├─ search
  └─ skill

crates/                 The framework and its assembly.
  ├─ proto              The wire schema and its generated types — `std` for
  │                     the daemon, `no_std` for a harness
  ├─ store              The keyspace: one KV primitive, and every interface
  │                     the runtime programs against, written over it
  ├─ runtime            The engine and the `Harness` seam — what you import to
  │                     build on crabtalk as a library
  ├─ transport          UDS + TCP socket layers, shared Transport enum
  ├─ client             Connection, typed RPC sugars, stream adapters
  ├─ crabtalk           Assembly: protocol handlers, composition, builder
  └─ berm               Crabtalk's side of berm — image loading, capabilities

apps/                   Products.
  ├─ agent              The daemon a general install runs: five KV methods
  │                     over crabdb, and the process that serves the runtime
  │                     over them. *A* product of the framework, not the
  │                     architecture.
  └─ crabup             Version manager for the ecosystem

harness/                One crate per harness, built two ways: `no_std` for
  ├─ os                 RV64 as a sandboxed ELF, `std` as compiled in.
  ├─ peers
  ├─ sessions
  └─ skill
```

Beside all of it sits `berm/` — the sandbox harnesses run in, plus its SDK
and its macro. It depends on no crate of ours, because crabtalk is one thing
that embeds it and deliberately not the only one; `berm/` and `harness/` leave
together when it does. See [RFC 0205](docs/src/rfcs/0205-berm.md).

## Where does my feature go?

| Question | Where |
|----------|-------|
| Would it make sense as a library without crabtalk? | lib/ |
| Does it shape an agent? | harness/ |
| Does it change a wire message? | crates/proto |
| Does it define what is persisted, or how it is keyed? | crates/store |
| Does it change execution, dispatch, or the `Harness` seam? | crates/runtime |
| Does it add a persistence backend? | the application — see apps/agent |
| Does it add a wire transport? | crates/transport |
| Does it need outbound network I/O or scheduling? | crates/crabtalk (system) |
| Does it bind a listener, or own process lifecycle? | the application — see apps/agent |
| Does it adapt a platform or speak to the daemon as a client? | crates/client |
| Does it install or update a crabtalk binary? | apps/crabup |
| **If none of these fit, challenge whether the feature should exist.** | |

## Boundary Contracts

- **Runtime** — never initiates I/O. It only responds. No sockets, timers, or listeners.
- **Runtime owns mechanics, clients own UX.** The store exposes session primitives (`create_session`, `append_session_messages`, `list_sessions`, `session_meta`, `search_sessions`), and the runtime exposes compaction and cancellation as mechanics the client invokes explicitly over the protocol — neither fires on its own. Discretionary lifecycle — `/clear`, `/new`, `/compact`, session selection, archival browsing, saved searches — is composed in the client from those primitives. See [RFC 0207](docs/src/rfcs/0207-store.md).
- **Crabtalk (library)** — never interprets tool semantics. It only routes. It is handed a `ClientMessage` and returns a stream; which endpoints a process serves, and where it advertises them, is the application's. Cron and config are system concerns (process-lifetime, not session-lifetime).
- **Client** — no dependency on runtime or model. Adapter-centric, not agent-centric.
- **Store** — the keyspace and the interfaces over it, and no engine. If it links a database or a search crate, the abstraction is wrong: which store to run is the application's choice.

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
- `Env` — engine environment: event broadcasting and the composite `Harness`
- `KVStorage` — the five methods a store implements; `Backend` bundles what the runtime needs, reached through `Config::Storage`
- `ToolDispatcher` — the trait the agent calls to execute a tool
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
