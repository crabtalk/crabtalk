# 0082 - Scoping

- Feature Name: Agent Scoping
- Start Date: 2026-03-22
- Discussion: [#82](https://github.com/crabtalk/crabtalk/pull/82)
- Crates: runtime, core

> **Updated by [0193 (Agent-Owned MCP)](0193-agent-owned-mcp.md) (2026-04-28).** `AgentScope.mcps` was removed: agents now embed their MCP server configurations by value, so MCP scoping is intrinsic to the agent's declaration and no separate allowlist is needed.

## Summary

A whitelist-based scoping system that restricts what an agent can access: tools and skills. Enforced at dispatch time and advertised in the system prompt. This is a security boundary, not a hint. MCP scoping is no longer part of `AgentScope` — see [0193](0193-agent-owned-mcp.md) for the replacement model.

Delegation is **not** scoped: crabtalk is a single-user runtime, and any
registered agent can delegate to any other. Multi-tenant identity-based access
control, if ever needed, belongs in a wrapper above the runtime, not inside
`AgentConfig`.

## Motivation

In multi-agent setups, a delegated sub-agent should not have the same
capabilities as the primary agent. A research agent doesn't need bash. Without
scoping, every agent has access to everything — which means a misbehaving or
confused agent can call tools it was never intended to use.

Scoping solves this by letting agent configs declare exactly what resources are
available. The runtime enforces it.

## Design

### AgentScope

```rust
pub struct AgentScope {
    pub tools: Vec<String>,     // empty = unrestricted
    pub skills: Vec<String>,    // empty = all skills
}
```

Empty list means unrestricted. Non-empty means only listed items are allowed. This is an inclusive whitelist, not a denylist. MCPs are not part of `AgentScope`: `AgentConfig.mcps: Vec<McpServerConfig>` makes the declaration itself the scope (RFC 0193).

### Whitelist computation

When an agent has any scoping (non-empty skills), the runtime computes a tool whitelist during `on_build_agent`:

1. Start with `BASE_TOOLS`: `bash`, `ask_user`, `read`, `edit` — always available.
2. If memory is enabled: add `recall`, `remember`, `memory`, `forget`.
3. If skills list is non-empty: add `skill` tool.
4. MCP tools the agent declared in `AgentConfig.mcps` are always included — declaration is the gate.

The computed whitelist replaces `config.tools`. Tools not on the list are invisible to the agent. The `delegate` tool is always available — delegation is not gated by scope.

### Prompt injection

A `<scope>` block is appended to the system prompt listing the agent's allowed
resources:

```xml
<scope>
skills: check-feeds, summarize
</scope>
```

This tells the agent what it can use. The agent doesn't need to guess or discover — its boundaries are stated upfront. MCP servers are listed separately in the resource-hints block from the agent's own `mcps` declaration.

### Enforcement

Scoping is enforced at two dispatch points:

- **Tool dispatch** — rejects tool calls not in the agent's tool whitelist.
- **Skill dispatch** — rejects skill names not in the agent's skill list.

MCP dispatch needs no explicit gate: the agent only sees the MCPs it declared, so calls outside that set are structurally impossible.

Enforcement happens at runtime, not just at prompt time. Even if the LLM
ignores the `<scope>` block and tries to call a restricted tool, the dispatch
layer rejects it.

### Sender restrictions

Not all base tools are available to all senders. `bash` is blocked for
non-CLI senders (gateway agents from Telegram, WeChat, etc.) because it
grants arbitrary shell access. `read` and `edit` have no sender
restriction — they are read-only or scoped mutations that are safe for
gateway agents. See [#67](https://github.com/crabtalk/crabtalk/issues/67).

### Delegate CWD isolation

When delegating parallel tasks, the orchestrating agent can assign each
sub-agent a separate working directory via the `cwd` field on `DelegateTask`.
Tools resolve relative paths against the conversation CWD, so isolated CWDs
prevent concurrent sub-agents from trampling each other's files. The `edit`
tool's unique-match requirement provides a second layer: if another agent
changed the file between read and edit, `old_string` won't match and the
edit fails — optimistic concurrency without locks.

### Default agent

The default agent (primary) has no scope restrictions — empty lists on all
three dimensions. Scoping is for sub-agents that need constrained access.

## Alternatives

**Denylist instead of whitelist.** List what's forbidden instead of what's
allowed. Rejected because allowlists are safer by default — a new tool or
server is inaccessible until explicitly granted. Denylists require updating
every time a new resource is added.

**Prompt-only scoping.** Tell the agent its restrictions in the prompt but
don't enforce at dispatch. Rejected because LLMs don't reliably follow
instructions — a determined or confused model will call tools it was told not
to. Enforcement must be at the dispatch layer.

## Unresolved Questions

- Should scoping support wildcard patterns (e.g. `mcp: search-*`)?
- Should scope violations be logged as security events for monitoring?
