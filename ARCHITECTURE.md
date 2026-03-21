## Architecture

Crabtalk is a daemon-based LLM agent orchestrator. Local-first, composable.

### Crates (`crates/`)

| Crate       | Package              | Role                                                                               |
| ----------- | -------------------- | ---------------------------------------------------------------------------------- |
| `core`      | `wcore`              | Agent, Runtime, Session, Protocol (protobuf), Model/Hook traits, ToolRegistry      |
| `daemon`    | `crabtalk-daemon`    | Daemon struct, DaemonHook (skills/MCP/memory/OS sub-hooks), event loop             |
| `transport` | `crabtalk-transport` | UDS + TCP socket layers                                                            |
| `model`     | `crabtalk-model`     | ProviderRegistry wrapping crabllm-provider (OpenAI/Anthropic/Google/Bedrock/Azure) |
| `gateway`   | `crabtalk-gateway`   | DaemonClient for platform adapters (UDS client, message types, streaming)          |
| `command`   | `crabtalk-command`   | Service command layer + proc macro codegen (`command-codegen`)                     |
| `cli`       | `crabtalk`           | CLI binary — thin UDS client with REPL, TUI, daemon control                        |

### Apps (`apps/`)

| App        | Role                                                             |
| ---------- | ---------------------------------------------------------------- |
| `hub`      | Package management library (manifest parsing, install/uninstall) |
| `telegram` | Telegram bot gateway → daemon via UDS                            |
| `search`   | Meta-search engine, optionally runs as MCP server                |

### Data Flow

```
Client (CLI/Telegram/etc) → UDS/TCP → Daemon event loop
  → Agent.step(): config + history + tools → Model.send()/stream()
  → Tool calls dispatched via ToolSender → DaemonHook.dispatch_tool()
  → Results back to agent via oneshot channel
```

### Key Types

- `Agent<M: Model>`: immutable definition + execution (step/run/run_stream)
- `Session`: conversation history container
- `Runtime<M, H>`: agents + sessions + tool dispatch
- `DaemonHook`: composites SkillHandler, McpHandler, Memory, OS hooks
- `DaemonEvent`: Message | ToolCall | Heartbeat | Shutdown
- `ToolRequest`: single tool call with reply channel
- Protocol: `ClientMessage` / `ServerMessage` (protobuf in `core/proto/crabtalk.proto`)
