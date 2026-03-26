## Architecture

Crabtalk is daemon-based LLM agent infrastructure.

### Crates (`crates/`)

| Crate       | Package              | Role                                                                               |
| ----------- | -------------------- | ---------------------------------------------------------------------------------- |
| `core`      | `wcore`              | Agent, Runtime, Session, Protocol (protobuf), Model/Hook traits, ToolRegistry      |
| `runtime`   | `crabtalk-runtime`   | Engine: RuntimeHook, tool dispatch, MCP, skills, memory, embeddable as library     |
| `daemon`    | `crabtalk-daemon`    | Thin server: DaemonHook wraps RuntimeHook, event loop, transport setup             |
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
  → Tool calls dispatched via ToolSender → RuntimeHook.dispatch_tool()
  → Results back to agent via oneshot channel
```

### Key Types

- `Agent<M: Model>`: immutable definition + execution (step/run/run_stream)
- `Session`: conversation history container
- `Runtime<M, H>`: agents + sessions + tool dispatch
- `RuntimeHook<B>`: engine hook — owns SkillHandler, McpHandler, Memory; dispatches tools
- `RuntimeBridge`: trait for server-specific tools (ask_user, delegate, session CWD)
- `DaemonHook`: wraps `RuntimeHook<DaemonBridge>`, adds event broadcasting
- `DaemonEvent`: Message | ToolCall | Heartbeat | Shutdown
- `ToolRequest`: single tool call with reply channel
- Protocol: `ClientMessage` / `ServerMessage` (protobuf in `core/proto/crabtalk.proto`)
