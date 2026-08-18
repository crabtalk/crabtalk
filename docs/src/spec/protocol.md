# Protocol

The protocol is the daemon's surface for everything outside its process. Clients speak it over a socket; a harness holding a `protocol:*` capability speaks the same one from inside a sandbox. There is no second vocabulary for harnesses.

## Addressing

A conversation is addressed by the pair `(agent, sender)`.

- `agent` names an agent registered in the daemon.
- `sender` is a client-provided string identifying the counterparty — `"user"`, `"event:deploy.done"`, a delegate id. Clients choose their own convention.

The pair is a conversation's only externally addressable name. **The wire carries no conversation identifier.** The `u64` the runtime keys its live map by is internal and never leaves the process.

| Message | Effect |
|---------|--------|
| `StreamMsg` | Append user content, run the agent, stream the response. |
| `SendMsg` | The same, returning one complete response rather than a stream. |
| `KillMsg` | Cancel the in-flight run, if any. |
| `CompactMsg` | Compact the current history into an archive. |
| `SteerSessionMsg` | Inject user content into a run already in flight. |

`StreamMsg.sender` is optional; when omitted the daemon resolves a default determined by the transport. `StreamMsg.guest` selects who speaks on this turn without changing whose conversation it is.

There is no working directory on the wire. `StreamMsg.cwd` was removed: the daemon does not read the user's filesystem, and a client that wants local context renders it into `content` itself.

A client declares no tools either. `SendMsg.tools` and `StreamMsg.tools` carried schemas the daemon advertised to the model and invoked back over the stream; both are reserved field numbers now, along with the forward event and its reply. A tool runs where the runtime does, which is what a harness is for.

## Capability groups

A harness reaches this surface through `crabtalk.protocol.call`, and what it may send is checked on decode against the group its declaration granted. The check is **default-deny**: a message type in no group reaches nothing, and a group the harness was not granted is the same.

- `protocol:read` — the catalogue, and nothing that spends tokens.
- `protocol:sessions` — ranked excerpts of the declaring agent's own past conversations.

Anything destructive, anything that answers on someone else's behalf, and anything whose payload is substantially a credential belongs to no group a third party can hold. Where a request carries a scope the harness should not choose — `SearchSessions.agent` — the host **overwrites** it rather than validating it. Refusing a wrong value would only teach the harness to send the right one.

## Transports

The daemon accepts client messages on its transports and produces a stream of server messages in response. Each message is handled independently, with no central event loop mediating between the transport and the operations.

## Entry point

Every transport (UDS, TCP, future additions) feeds `ClientMessage` values into the same dispatch callback. The callback spawns a Tokio task per message and polls the resulting stream, forwarding each `ServerMessage` back to the transport's reply channel. When the stream ends or the reply channel closes, the task terminates.

Concurrency is unbounded at this layer: nothing throttles or serializes incoming messages before they reach their handler.

## Dispatch function

`Server::dispatch(ClientMessage) -> Stream<ServerMessage>` is the single entry into the daemon's operations. It inspects the `ClientMessage` variant and routes to the corresponding method on the `Server` trait.

It is also the door a harness comes back through. A harness granted a `protocol:*` capability sends the same `ClientMessage` a client would and receives the same reply — one vocabulary, not two. What it is *allowed* to send is checked on decode against the group its declaration granted, and default-deny: a message named in no group reaches nothing.

- Request-response operations (`ping`, `kill_conversation`, `compact_conversation`, administrative calls) yield exactly one `ServerMessage`.
- Streaming operations (`stream`, `subscribe_events`) yield many `ServerMessage` values over time.
- Unknown or empty messages yield a single error response.

The function is defined once in the core `Server` trait. Any implementor — the daemon, a test harness, a future alternative server — routes client messages the same way.

## No central event loop

There is no serializing queue, no `DaemonEvent` enum, and no actor that owns mutation. Operations reach into shared state directly and hold locks for the duration of the critical section.

Shared state is protected by `parking_lot::Mutex` or `parking_lot::RwLock`. Event bus subscriptions and the live session registry each live behind their own lock. Locks are acquired, the work is done, and the lock is released. Ordering between operations is whatever Tokio's scheduler produces.

## Ordering guarantees

Within a single conversation, message ordering is total: `StreamMsg` appends to history in the order the daemon receives them. Clients that require strict ordering for a conversation are responsible for serializing their own sends.

Between conversations, no ordering is guaranteed. Two `StreamMsg` values addressed to different `(agent, sender)` pairs may run in either order regardless of arrival time.

## Cancellation

`KillMsg` cancels the in-flight run for its `(agent, sender)` pair. Cancellation propagates through the runtime to the active agent step, interrupting tool calls and LLM requests at the next await point. Already-emitted `ServerMessage` values are not retracted.

A cancelled conversation remains valid. The next `StreamMsg` for the same pair resumes against the history as it existed at the point of cancellation.

## Event bus

The event bus is a subscription table, not a router. `publish(source, payload)` iterates subscriptions, invokes the `fire` callback for each match inline, and removes any subscription marked `once`. The callback fires under the bus's lock; implementations must not reacquire it.

The bus has no queue and no scheduler. Fan-out is as fast as the callback runs for each matching subscription.
