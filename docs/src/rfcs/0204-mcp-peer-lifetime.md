# 0204 - MCP Peer Lifetime

- Feature Name: MCP Peer Lifetime
- Start Date: 2026-08-15
- Discussion: TBD
- Crates: mcp, core, crabtalk, tui
- Updates: [0193 (Agent-Owned MCP)](0193-agent-owned-mcp.md)

## Summary

An MCP peer is a process, and this RFC gives it a lifetime of its own rather than borrowing the declaration's. Credentials join the dedup fingerprint, so a peer is never shared across a trust boundary. A peer that stops answering now says so instead of reporting `Connected` forever. `ReconnectMcp` brings one back on request. And a peer starts on an agent's first MCP tool call and stops once it goes idle, instead of existing for as long as some agent has the config in its TOML.

## Motivation

RFC 0193 made MCPs agent-owned and deduped identical configs down to one process. Both decisions hold. What did not survive contact with a multi-tenant host is the assumption underneath them: that a declaration and a process are the same thing, and that structural identity is `(command, args, env, url)`.

**Credentials were not part of identity.** `fingerprint` hashed `env` precisely because it carries a stdio server's secrets — but `auth`, the HTTP transport's equivalent, was left out. Two agents naming the same URL with different bearer tokens hashed identically, so the second refcount-bumped onto the first one's peer and its tool calls went out under the first agent's header. On a single-user install that is invisible. On a shared host it is a credential leak, and it is the one config that dedup must never collapse.

**Peers were tied to registration.** `register_for_agent` spawned on first reference and only the last `unregister_for_agent` tore down. For a CLI with a handful of agents that is fine — everything is warm and nothing is wasted. Host ten thousand tenants and it means a child process for every MCP any of them ever mentioned, alive from boot, whether or not a model ever called one. Dedup does not rescue this: the interesting servers are per-tenant credentialed, and with credentials now correctly part of identity, those are exactly the configs that *cannot* share a peer. The process count scales with tenants, not with distinct tools.

**Failure was invisible.** Nothing wrote back to peer state after the initial connect. An expired token or a dead child surfaced as an error string handed to the model; `states()` kept reporting `Connected` and `McpEventKind::Failed` never fired again. A client watching the lifecycle stream was blind to the single condition it would want to act on, which also made "let the client decide whether to retry" — 0193's stated position — unimplementable in practice.

## Design

### Credentials are identity

`fingerprint` hashes `(command, args, env, url, auth)`. One peer sends one `Authorization` header on behalf of every agent sharing it, so agents holding different tokens get different peers.

This has a second effect worth naming: rotating a token is now a real reconnect through the existing surface. A new token is a new fingerprint, so `UpsertMcp` with fresh credentials tears the old peer down and stands a new one up. Before, the rotated config hashed to the same fingerprint, took the refcount fast path, and left the peer running with the expired header while the daemon reported success.

It also exposed a latent leak. `register_for_agent` already handled a claim moving between fingerprints, but it only removed the ref — a peer whose ref set went empty stayed in the map, unreachable from `by_owner`, so `unregister_for_agent` could never find it and the process lived until daemon shutdown. That path was rare when only a command or env edit reached it. Putting `auth` in the fingerprint makes every token rotation take it, which would have turned a rare leak into a routine one. The claim move now mirrors `unregister_for_agent`: last ref out, peer torn down.

### Declaration and process have separate lifetimes

This is the data-model change the rest rests on. A `PeerEntry` is a *declaration*. It appears when an agent registers the config, survives eviction, and is removed only when the last owner unregisters. Whether a process is currently running behind it is `state.status`, and nothing else.

- **Registration records.** `register_for_agent` inserts or refcounts the declaration and returns. It spawns nothing, which is what removes the boot-time process fleet.
- **Dispatch spawns.** `dispatch_mcp` calls `ensure_connected` before anything else. The `mcp` meta-tool is the only door a model has — listing and calling both arrive here — so one call covers both. The ordering is load-bearing: `allowed()` reads the tool list a peer only reports once connected, so connecting second would tell an agent it has no tools it could ever reach.
- **The reaper stops.** A background task ticks at `idle_timeout / 4`, so a peer outlives its deadline by at most a quarter of it. It holds a `Weak`, so it ends with the handler rather than keeping it alive. `Failed` peers are reaped too — the transport broke, but the process is still running and still costs a slot.

Connects run concurrently across an agent's MCPs and are serialised per peer by a gate on the entry, so two agents reaching a shared peer wait on one spawn instead of racing two processes under the same id.

**`Failed` peers are not retried by `ensure_connected`.** A peer with a wrong command would otherwise burn a connect timeout on every turn, forever. It becomes eligible again once the reaper ages it back to `Disconnected`, which bounds the retry rate to the idle timeout with no backoff machinery, and `ReconnectMcp` exists to retry sooner on demand.

**`UpsertMcp` still connects eagerly.** Registration is mechanical and lazy, but a human who just typed a config should learn about a bad command or a stale token there and then, not inside some later tool call. So the daemon-side handler calls `ensure_connected` explicitly after the write. Mechanism in the handler, policy at the edge — RFC 0189's split, applied here.

### Health is state, not a return value

`McpBridge::call` flattened five distinct failures into one `Err(String)`, and two of them mean opposite things: `mcp tool error` is the far side answering with a rejection, so the connection is fine, while `mcp call failed` is the call never landing. Marking a peer failed on the flattened error would have torn down a healthy peer every time a model passed a bad argument.

So the return type is `Result<String, CallError>` with `Rejected` and `Transport`, rendering through `Display` to the exact strings the model already saw. Only `Transport` touches peer state.

The call itself moved onto `McpHandler`. The one place that learns a peer is dead had no access to the state clients read; now the bridge owns connections and the handler owns observable state. `allowed()` returns `Fingerprint` rather than a hex string, so the typed id runs to the edge and stringifies once at the bridge boundary — the handler no longer has to parse its own encoding back to find a peer.

Marking a peer failed preserves its tool list. The tools describe what the peer exports, not whether it is reachable; clearing them makes the next call fail with `tool not available`, which disguises a dead connection as a scoping error. Eviction clears them, because by then nothing is running.

### Reconnect

`ReconnectMcpMsg { name, agent }` joins the `ClientMessage` oneof at tag 54, mirroring `DeleteMcpMsg` field for field. It answers with `McpInfo` rather than `Pong`, the way `UpsertMcp` does, so a caller gets post-reconnect status and any error in one round trip instead of following up with `ListMcps`.

Reconnecting is per-peer, not per-claim: a process shared by several agents comes back once for all of them, and every owner sees the lifecycle events under its own name for that MCP.

The config comes from the peer, not the caller. `PeerEntry` stores the `McpServerConfig` it was spawned from, env overlay already applied. This matters because `McpHook` applies a daemon-wide overlay at registration, so the tracked fingerprint is of the *effective* config — reconnecting from the agent's stored config would compute a different fingerprint and strand the peer under an id that no longer describes it. Storing what was actually run removes the whole class of drift, and leaves the wire message carrying nothing but identity.

Changing a config is explicitly not this operation. That is `UpsertMcp`, which mints a new fingerprint and therefore a new peer.

### Configuration

```toml
[mcp]
idle_timeout = 1800
```

Seconds a peer may sit unused before the reaper stops it. Zero disables eviction and restores 0193's behaviour of peers living until their last agent unregisters. A deployment picks this because the right answer differs between a laptop, where warm peers cost nothing that matters, and a shared host, where process count is the binding constraint.

## Updates

### 0193 - Agent-Owned MCP

The fingerprint tuple gains `auth` — 0193 specifies `(command, args, env, url)`.

"`register_for_agent(agent, cfg)` increments the refcount, spawning if first" no longer holds; it records only. The **Lifecycle interactions** section reads the same way throughout — agent create, update, daemon startup, and daemon reload all describe new fingerprints spawning. They now register declarations, and the first dispatch spawns. Agent delete and rename are unchanged.

**Out of scope** ruled that "whether a client retries is a client decision." That intent survives and is finally actionable: the daemon still never auto-restarts, but a failed peer is now visible on the lifecycle stream and in `ListMcps`, and `ReconnectMcp` gives the client something to call. The reaper ages `Failed` back to `Disconnected`, which makes the following dispatch a natural retry — a bound on retry frequency, not an auto-restart policy.

The **Migration** section's one-shot inlining of the global `[mcps]` registry has been removed, as that section itself anticipated ("the migration code path is dead and gets removed in a follow-up cleanup commit").

## Alternatives

**Sweep for idle peers during dispatch instead of running a reaper.** No background task, no tick interval. Rejected because it inverts the case that matters: a host with thousands of idle tenants is precisely one where nothing dispatches, so nothing would ever sweep. Eviction has to be driven by time passing, not by traffic.

**Retry failed peers automatically with backoff.** Rejected as machinery for a bound the design already has. The idle timeout is a retry interval for free, and it is a number the operator already chose.

**Per-agent idle timeout.** Rejected because a peer is shared. Two agents with different timeouts on one process leaves no honest answer to whose wins, and the tie-break would be invisible to both.

**Keep the config out of `PeerEntry` and pass it to reconnect.** Rejected: the effective config is the handler's own product, not the caller's, and reproducing it at the call site means duplicating the env-overlay rule wherever a reconnect is triggered.

## Unresolved Questions

- None of this has been exercised against a live daemon. The reaper's timing, the per-peer gate under real concurrency, and the `Failed` → `Disconnected` → retry cycle are reasoned through rather than observed.
- The first call after an eviction pays a connect, bounded by the existing 30s `MCP_CONNECT_TIMEOUT`. What that actually costs a conversation has not been measured, and it is the number that would justify moving `idle_timeout`'s default.
- `McpHook`'s `env_overlay` is daemon-wide and backfills any key a tenant's MCP did not set. Harmless for one user; on a shared host it means a daemon-level secret can reach a tenant process. Untouched here.
