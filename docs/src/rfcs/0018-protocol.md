# 0018 - Protocol

- Feature Name: Wire Protocol
- Start Date: 2026-03-27
- Discussion: [#18](https://github.com/crabtalk/crabtalk/pull/18)
- Crates: core

## Summary

A protobuf-based wire protocol defining all client-server communication for the
crabtalk daemon, with a `Server` trait for dispatch and a `Client` trait for
typed request methods.

## Motivation

The daemon mediates between multiple clients (CLI, Telegram, web) and multiple
agents. A well-defined wire protocol decouples client and server implementations
and makes the contract explicit. Protobuf was chosen for compact binary
encoding, language-neutral schema, and generated code via `prost`.

## Design

### Wire messages (`crabtalk.proto`)

Two top-level envelopes using `oneof`:

**ClientMessage** — 15 variants:

| Variant | Purpose |
|---------|---------|
| `Send` | Run agent, return complete response |
| `Stream` | Run agent, stream response events |
| `Ping` | Keepalive |
| `Sessions` | List active sessions |
| `Kill` | Close a session |
| `GetConfig` | Read daemon config |
| `SetConfig` | Replace daemon config |
| `Reload` | Hot-reload runtime |
| `SubscribeEvents` | Stream agent events |
| `ReplyToAsk` | Answer a pending `ask_user` prompt |
| `GetStats` | Daemon stats |
| `CreateCron` | Create cron entry |
| `DeleteCron` | Delete cron entry |
| `ListCrons` | List cron entries |
| `Compact` | Compact session history |

**ServerMessage** — 11 variants:

| Variant | Purpose |
|---------|---------|
| `Response` | Complete agent response |
| `Stream` | Streaming event (see below) |
| `Error` | Error with code and message |
| `Pong` | Keepalive ack |
| `Sessions` | Session list |
| `Config` | Config JSON |
| `AgentEvent` | Agent event (for subscriptions) |
| `Stats` | Daemon stats |
| `CronInfo` | Created cron entry |
| `CronList` | All cron entries |
| `Compact` | Compaction summary |

### Streaming events

`StreamEvent` is itself a `oneof` with 8 variants representing the lifecycle of
a streamed agent response:

- `Start { agent, session }` — stream opened.
- `Chunk { content }` — text delta.
- `Thinking { content }` — thinking/reasoning delta.
- `ToolStart { calls[] }` — tool invocations beginning.
- `ToolResult { call_id, output, duration_ms, is_error }` — single tool result. `is_error` signals the handler reported failure; `output` carries the text in either case so clients can render it. UIs use the flag to style errors distinctly; agents can use it for retry decisions without string-matching on error messages.
- `ToolsComplete` — all pending tool calls finished.
- `AskUser { questions[] }` — agent needs user input.
- `End { agent, error }` — stream closed (error is empty on success).

The client reads `StreamEvent`s until it receives `End`, which is the terminal
sentinel.

**Tool result ordering.** When a single agent step produces N tool calls, the runtime dispatches them concurrently and emits `ToolResult` events in **completion order** — fast tools are reported as soon as they finish, slow siblings report later. The event stream is therefore not ordered by the call index in `ToolStart.calls[]`. Clients correlate by `call_id`, which is the primary key; do not assume positional alignment with the `ToolStart` call list.

### Agent events

`AgentEventMsg` carries a `kind` enum (`TEXT_DELTA`, `THINKING_DELTA`,
`TOOL_START`, `TOOL_RESULT`, `TOOLS_COMPLETE`, `DONE`) plus agent name, session
ID, content, and timestamp. Used by `SubscribeEvents` for live monitoring of all
agent activity across sessions. For `TOOL_RESULT` events, the `tool_is_error` field mirrors the streaming protocol's `is_error` — monitoring clients use it to aggregate error rates per tool type without parsing output strings.

`AgentEventMsg` overlaps with `StreamEvent` — both represent the agent execution
lifecycle. `StreamEvent` is the per-request streaming format (rich, typed
variants). `AgentEventMsg` is the cross-session monitoring format (flat, single
struct with a kind tag). The duplication exists because monitoring clients need a
simpler, uniform shape to filter and display events from multiple agents.

### Server trait

One async method per `ClientMessage` variant. Implementations receive typed
request structs and return typed responses:

```rust
trait Server: Sync {
    fn send(&self, req: SendMsg) -> Future<Output = Result<SendResponse>>;
    fn stream(&self, req: StreamMsg) -> Stream<Item = Result<StreamEvent>>;
    fn ping(&self) -> Future<Output = Result<()>>;
    // ... one method per operation
}
```

The provided `dispatch(&self, msg: ClientMessage) -> Stream<Item =
ServerMessage>` method routes a raw `ClientMessage` to the correct handler.
Request-response operations yield exactly one `ServerMessage`; streaming
operations yield many. Errors are mapped to `ErrorMsg { code, message }` using HTTP status codes with
their standard semantics: 400 (bad request), 404 (not found), 500 (internal
error).

### Client trait

Two required transport primitives:

- `request(ClientMessage) -> Result<ServerMessage>` — single round-trip.
- `request_stream(ClientMessage) -> Stream<Item = Result<ServerMessage>>` —
  raw streaming read.

Typed provided methods (`send`, `stream`, `ping`, `get_config`, `set_config`)
handle message construction, response unwrapping, and sentinel detection. The
`stream()` method consumes events via `take_while` until `StreamEnd` and maps
each frame through `TryFrom<ServerMessage>` for type-safe event extraction.

### Conversions (`message::convert`)

`From` impls wrap typed messages into envelopes (`SendMsg -> ClientMessage`,
`SendResponse -> ServerMessage`). `TryFrom` impls unwrap in the other direction,
returning an error for unexpected variants. This keeps call sites clean — no
manual enum construction.

## Alternatives

**JSON over WebSocket.** Simpler to debug with `curl`, but larger payloads and
no schema enforcement. Protobuf catches schema mismatches at compile time.

**gRPC service definitions.** Would provide streaming and code generation out of
the box, but brings HTTP/2, tower middleware, and tonic as dependencies. The
current approach is lighter: raw protobuf frames over a length-prefixed stream,
with hand-written trait dispatch.

**Separate request/response ID correlation.** The protocol is connection-scoped
and sequential — one outstanding request per connection at a time. This is a
fundamental design constraint: clients must wait for a response before sending
the next request. No need for request IDs or multiplexing. If multiplexing is
needed later, it belongs in the transport layer, not the protocol.

## Unresolved Questions

- Should the protocol negotiate a version on connect to detect client/server
  mismatches?
- Should `StreamEnd` carry structured error information (code + message) instead
  of a plain string?
- Should there be a `ClientMessage` variant for subscribing to a specific
  session's events rather than all events?
