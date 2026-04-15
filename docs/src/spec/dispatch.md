# Dispatch

The daemon accepts client messages on its transports and produces a stream of server messages in response. Each message is handled independently, with no central event loop mediating between the transport and the operations.

## Entry point

Every transport (UDS, TCP, future additions) feeds `ClientMessage` values into the same dispatch callback. The callback spawns a Tokio task per message and polls the resulting stream, forwarding each `ServerMessage` back to the transport's reply channel. When the stream ends or the reply channel closes, the task terminates.

Concurrency is unbounded at this layer: nothing throttles or serializes incoming messages before they reach their handler.

## Dispatch function

`Server::dispatch(ClientMessage) -> Stream<ServerMessage>` is the single entry into the daemon's operations. It inspects the `ClientMessage` variant and routes to the corresponding method on the `Server` trait.

- Request-response operations (`ping`, `kill_conversation`, `compact_conversation`, administrative calls) yield exactly one `ServerMessage`.
- Streaming operations (`stream`, `subscribe_events`) yield many `ServerMessage` values over time.
- Unknown or empty messages yield a single error response.

The function is defined once in the core `Server` trait. Any implementor — the daemon, a test harness, a future alternative server — routes client messages the same way.

## No central event loop

There is no serializing queue, no `DaemonEvent` enum, and no actor that owns mutation. Operations reach into shared state directly and hold locks for the duration of the critical section.

Shared state is protected by `parking_lot::Mutex` or `parking_lot::RwLock`. Event bus subscriptions, conversation working-directory overrides, pending `ask_user` replies, and cron state each live behind their own lock. Locks are acquired, the work is done, and the lock is released. Ordering between operations is whatever Tokio's scheduler produces.

## Ordering guarantees

Within a single conversation, message ordering is total: `StreamMsg` appends to history in the order the daemon receives them. Clients that require strict ordering for a conversation are responsible for serializing their own sends.

Between conversations, no ordering is guaranteed. Two `StreamMsg` values addressed to different `(agent, sender)` pairs may run in either order regardless of arrival time.

## Cancellation

`KillMsg` cancels the in-flight run for its `(agent, sender)` pair. Cancellation propagates through the runtime to the active agent step, interrupting tool calls and LLM requests at the next await point. Already-emitted `ServerMessage` values are not retracted.

A cancelled conversation remains valid. The next `StreamMsg` for the same pair resumes against the history as it existed at the point of cancellation.

## Event bus

The event bus is a subscription table, not a router. `publish(source, payload)` iterates subscriptions, invokes the `fire` callback for each match inline, and removes any subscription marked `once`. The callback fires under the bus's lock; implementations must not reacquire it.

The bus has no queue and no scheduler. Fan-out is as fast as the callback runs for each matching subscription.
