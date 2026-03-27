# Contributing

Crabtalk is a workspace of crates and apps. The daemon is the product — everything
else either powers it or connects to it.

## Layering

```
Layer 0 ─ Foundation
  └─ core (wcore)        Agent, Session, Runtime, Protocol, Hook, ToolRegistry

Layer 1 ─ Backends (independent of each other)
  ├─ model               ProviderRegistry (OpenAI, Anthropic, Google, Bedrock, Azure)
  ├─ transport            UDS + TCP socket layers
  └─ command              Service management (systemd), proc macro codegen

Layer 2 ─ Engine
  └─ runtime              RuntimeHook, tool dispatch, MCP, skills, memory

Layer 3 ─ Server
  └─ daemon               Event loop, transport setup, cron, config, hot reload

Layer 4 ─ Adapters
  ├─ gateway              DaemonClient, message types for platform adapters
  ├─ cli                  REPL, TUI, daemon control
  └─ apps/                telegram, search, hub, wechat, outlook
```

## Where does my feature go?

| Question | Crate |
|----------|-------|
| Does it define agent behavior, protocol types, or tool schemas? | core |
| Does it add or configure an LLM provider? | model |
| Does it add a wire transport? | transport |
| Does it add a tool the agent can call, a skill, or memory? | runtime |
| Does it need network I/O, scheduling, or process lifecycle? | daemon |
| Does it adapt a platform or parse bot commands? | gateway |
| Does it add a CLI command or TUI feature? | cli |
| **If none of these fit, challenge whether the feature should exist.** | |

## Boundary Contracts

### Daemon — Runtime

The daemon constructs the runtime and feeds it messages. Tool calls come back
through the event channel. `RuntimeBridge` is the seam between embedded and
daemon modes.

**Runtime never initiates I/O** — it only responds. If your feature needs to
open a socket, schedule a timer, or listen for connections, it does not belong
in the runtime.

**Daemon never interprets tool semantics** — it only routes. If your feature
needs to understand what a tool does or modify agent prompts, it does not belong
in the daemon.

Cron and config are daemon concerns — they're process-lifetime, not
session-lifetime.

### Gateway — Runtime

Gateway does not depend on runtime or model. It's adapter-centric, not
agent-centric. If your feature involves LLM logic, tool dispatch, or agent
behavior, it does not belong in the gateway.

### Core — Everything

Core is the foundation. It defines traits and types but never implements
server-specific behavior. If your change to core requires pulling in runtime
or daemon dependencies, the abstraction is wrong.

## Data Flow

```
Client (CLI/Telegram/etc) → UDS/TCP → Daemon event loop
  → Agent.step(): config + history + tools → Model.send()/stream()
  → Tool calls dispatched via ToolSender → RuntimeHook.dispatch_tool()
  → Results back to agent via oneshot channel
```

## Key Types

- `Agent<M: Model>` — immutable definition + execution (step/run/run_stream)
- `Session` — conversation history container
- `Runtime<M, H>` — agents + sessions + tool dispatch
- `RuntimeHook<B>` — engine hook: skills, MCP, memory, tool routing
- `RuntimeBridge` — trait for server-specific tools (ask_user, delegate, session CWD)
- `DaemonHook` — wraps `RuntimeHook<DaemonBridge>`, adds event broadcasting
- `DaemonEvent` — Message | ToolCall | Shutdown
- `ToolRequest` — single tool call with reply channel
- Protocol — `ClientMessage` / `ServerMessage` (protobuf)

## Features

Features and their design rationale are documented as RFCs in the
[development book](docs/).
