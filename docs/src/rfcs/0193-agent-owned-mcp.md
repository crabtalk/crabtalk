# 0193 - Agent-Owned MCP

- Feature Name: Agent-Owned MCP
- Start Date: 2026-04-28
- Discussion: TBD
- Crates: core, mcp, crabtalk, runtime
- Updates: [0082 (Scoping)](0082-scoping.md), [0135 (Agent-First)](0135-agent-first.md), [0190 (MCP Lifecycle)](https://github.com/crabtalk/crabtalk/pull/192)

## Summary

Agents own their MCP servers by value, not by name reference into a daemon-global registry. `AgentConfig.mcps` becomes `Vec<McpServerConfig>` — every agent carries the full configuration of every MCP it uses. The daemon's job shrinks to "spawn what agents declare, dedup identical processes, route tool calls per agent." `Storage::{list,upsert,delete}_mcp` and `crabtalkd mcp` go away. Forking an agent now means copying one config; the new owner gets a self-contained, runnable artifact.

## Motivation

The current model treats MCPs as a daemon-level resource that agents reference by name. That made sense when crabtalk was a single-user CLI managing a fixed fleet of tools. It doesn't fit where the runtime is going.

**Forkability is broken.** RFC 0135 framed agents as the unit users see and share — sessions are plumbing, agents are the artifact. Cloud workflows extend that: an agent should be a forkable thing, like a GitHub repo. Today, forking an agent's TOML doesn't fork its MCPs; the fork lands on a daemon that may or may not have a server registered under the same name, with the same args, with the same env. The agent reference is a dangling pointer until someone manually re-registers the missing pieces.

**Namespace pollution is artificial.** Two agents that want the same logical MCP with different env (e.g., one read-only token, one admin token) must register two differently-named entries in a global flat namespace. The bridge's `tool_cache: BTreeMap<String, Tool>` then logs-and-skips conflicts when both expose `web_search`. None of that pollution is intrinsic to MCP; it's a consequence of the registry shape.

**The allowlist is a workaround for ownership.** `AgentConfig.mcps: Vec<String>` (RFC 0082) gates which global entries an agent may dispatch to. It exists because the registry is shared. If agents own their MCPs, allowlists become tautological — the agent only dispatches to what it declared.

The cloud target makes this acute. Cloud will import `crabtalk` as a library and host one agent per tenant (or per agent instance). A daemon-global registry on a multi-tenant host either leaks configurations across tenants or forces the cloud layer to maintain its own per-tenant overlay on top of the registry. Either way the global registry is wrong — the right shape is "agent has its MCPs," and the cloud's secret/canonical layer can compose forkable templates above that.

## Design

### Data model

```rust
struct AgentConfig {
    // …
    mcps: Vec<McpServerConfig>,  // was Vec<String>
}
```

Embedded by value. No enum wrapper, no separate "decl" type. The agent's TOML carries every field of every MCP it depends on.

`Storage` loses `list_mcps`, `upsert_mcp`, `delete_mcp`. The protocol RPCs `ListMcps`, `UpsertMcp`, `DeleteMcp` stay — they shift meaning from "manage the global registry" to "list MCPs declared by any registered agent" / "modify an agent's MCPs in place" / "remove an MCP from an agent's config." Implemented by reading and writing through the agent's config rather than a separate table.

### Daemon-side dedup

The daemon never spawns the same MCP twice. Two agents declaring `command="github-mcp", args=[...], env={TOKEN: "abc"}` share one peer process. Different `args` or `env` → separate processes. Identity is structural, not by name.

`McpHandler` keys peers by **fingerprint** — a stable hash of `(command, args, env, url)`. The state map becomes `BTreeMap<Fingerprint, McpServerEntry>` where each entry refcounts the agents that declared it. `register_for_agent(agent, cfg)` increments the refcount, spawning if first; `unregister_for_agent(agent, fingerprint)` decrements, tearing down at zero.

The lifecycle event broadcast from RFC 0190 (PR #192) still applies: `Connecting` / `Connected` / `Failed` / `Disconnected` are emitted per fingerprint, not per name. The event payload identifies the server by fingerprint plus the set of agents that own a reference to it.

### Per-agent tool namespace

The bridge stops sharing a flat `tool_cache`. Two agents declaring different MCPs that both expose a `web_search` tool no longer collide — the dispatcher resolves `(agent, tool_name)` to the right peer through the agent's declared fingerprints.

Concretely: `McpBridge` keeps the per-fingerprint peer map but drops the global tool cache. Tool lookup walks the agent's fingerprints in declaration order and returns the first match. `McpHook::dispatch` already has the agent context; it now uses the agent's declared MCPs directly instead of consulting an `AgentScope.mcps` allowlist.

### Lifecycle interactions

- **Agent create / update.** `Runtime::create_agent` and `update_agent` walk the config's `mcps` list, calling `McpHandler::register_for_agent(agent, cfg)` for each. New fingerprints spawn; existing fingerprints just bump the refcount.
- **Agent delete.** Walks the agent's `mcps`, calls `unregister_for_agent` for each. Peers with refcount=0 are torn down. `Disconnected` events fire.
- **Agent rename.** Refcounts move from `old_name` to `new_name`. No spawn/teardown.
- **Daemon startup.** Storage rebuilds agents one by one; each `register_for_agent` call walks the same dedup path. No special "load global MCPs" phase.
- **Daemon reload.** Already rebuilds agents (RFC 0189-era refactor). Same path. New configs trigger spawns; removed fingerprints trigger teardowns.

### Where secrets are not

The daemon stores literal `McpServerConfig` values. There is no placeholder syntax, no resolver trait, no interpolation in this codebase. If a value looks like `${TAVILY_KEY}`, the daemon spawns a process with that literal string in the environment.

The "canonical with placeholders / materialized with values" split lives in whatever sits above the daemon. Cloud's control plane holds canonical agent configs (with `${TAVILY_KEY}`), resolves against the tenant's vault, and writes the resolved config to the daemon-as-library it owns for that tenant. Forks copy the canonical, never the resolved.

This keeps the forkability invariant — shareable artifacts carry structure, not values — while keeping the daemon secret-unaware.

## Migration

`AgentConfig.mcps` is a breaking field type change (`Vec<String>` → `Vec<McpServerConfig>`). Existing configs on disk need a one-shot migration:

1. On daemon startup, if any agent's `mcps` is `Vec<String>` (detected via serde), look each name up in the existing `mcps.toml` (or whatever Storage held the global registry), inline the `McpServerConfig`, and rewrite the agent's TOML.
2. After every agent has been migrated, delete the global `mcps.toml`.

The migration runs once. After the first startup on the new code, configs are uniformly the new shape; the migration code path is dead and gets removed in a follow-up cleanup commit.

`Storage::list_mcps` / `upsert_mcp` / `delete_mcp` are removed from the trait. Implementations — `FsStorage`, `MemStorage` — drop the corresponding files/fields. The protocol RPCs `ListMcps` / `UpsertMcp` / `DeleteMcp` stay on the wire; their handlers are rewritten to operate on agent configs.

`AgentScope.mcps` (RFC 0082) is removed. The scoping struct still gates `tools` and `skills`; MCP scoping is now intrinsic to the agent's declaration.

## Alternatives considered

**Keep the global registry, add per-agent overrides.** Allow `AgentConfig.mcps` to carry inline overrides on top of name references. Rejected because it doubles the configuration surface — every consumer has to handle "which wins, the override or the registry?" — without solving forkability. Forking an agent still depends on the destination daemon having the right names registered.

**SecretResolver trait in this repo.** Earlier draft. Cut because the daemon can stay secret-unaware: cloud handles canonical-vs-resolved at its control plane and only writes resolved configs into the daemon. Adding a trait here for a default that just reads env vars is complexity for a problem we don't have.

**Generic on `Daemon` for the resolver.** Even if a resolver lived in this repo, adding a second type parameter to `Daemon<P>` compounds complexity per the no-generics-for-future-use rule. Not worth it for a hypothetical hook.

**Plugin-provided MCPs as agent templates.** Plugins are being removed, so this collapses. Future plugin-like artifacts (if any) will compose at the agent level rather than at a separate MCP-registry level.

## Out of scope

- Secret resolution, vaulting, or `${VAR}` interpolation. Cloud's problem, not the daemon's.
- Auto-restart behavior for failed peers. Lifecycle events from PR #192 surface failures; whether a client retries is a client decision.
- Discovery of port-file MCPs. Today `McpHandler` auto-connects services that drop a `*.port` file under `~/.crabtalk/run/`. That mechanism continues to work, but discovered servers now register against a synthetic per-process "discovery agent" (or are exposed only on the daemon-internal dispatch path) — the exact shape is a follow-up.
- Plugin MCPs. Plugins are being removed; no migration needed.
