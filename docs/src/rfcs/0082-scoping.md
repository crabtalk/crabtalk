# 0082 - Scoping

- Feature Name: Agent Scoping
- Start Date: 2026-03-22
- Discussion: [#82](https://github.com/crabtalk/crabtalk/pull/82)
- Crates: runtime, core

## Summary

A whitelist-based scoping system that restricts what an agent can access: tools,
skills, MCP servers, and delegation targets. Enforced at dispatch time and
advertised in the system prompt. This is a security boundary, not a hint.

## Motivation

In multi-agent setups, a delegated sub-agent should not have the same
capabilities as the primary agent. A research agent doesn't need bash. A
summarizer doesn't need to delegate to other agents. Without scoping, every
agent has access to everything — which means a misbehaving or confused agent
can call tools it was never intended to use.

Scoping solves this by letting agent configs declare exactly what resources are
available. The runtime enforces it.

## Design

### AgentScope

```rust
pub struct AgentScope {
    pub tools: Vec<String>,     // empty = unrestricted
    pub members: Vec<String>,   // empty = no delegation
    pub skills: Vec<String>,    // empty = all skills
    pub mcps: Vec<String>,      // empty = all MCP servers
}
```

Empty list means unrestricted. Non-empty means only listed items are allowed.
This is an inclusive whitelist, not a denylist.

### Whitelist computation

When an agent has any scoping (non-empty skills, mcps, or members), the runtime
computes a tool whitelist during `on_build_agent`:

1. Start with `BASE_TOOLS`: `bash`, `ask_user` — always available.
2. If memory is enabled: add `recall`, `remember`, `memory`, `forget`.
3. If skills list is non-empty: add `skill` tool.
4. If mcps list is non-empty: add `mcp` tool.
5. If members list is non-empty: add `delegate` tool.

The computed whitelist replaces `config.tools`. Tools not on the list are
invisible to the agent.

### Prompt injection

A `<scope>` block is appended to the system prompt listing the agent's allowed
resources:

```xml
<scope>
skills: check-feeds, summarize
mcp servers: search
members: researcher, writer
</scope>
```

This tells the agent what it can use. The agent doesn't need to guess or
discover — its boundaries are stated upfront.

### Enforcement

Scoping is enforced at four dispatch points:

- **`dispatch_tool`** — rejects tool calls not in the agent's tool whitelist.
- **`dispatch_skill`** — rejects skill names not in the agent's skill list.
- **`dispatch_mcp`** — filters MCP server list to allowed servers.
- **`dispatch_delegate`** — rejects delegation to agents not in the members
  list.

Enforcement happens at runtime, not just at prompt time. Even if the LLM
ignores the `<scope>` block and tries to call a restricted tool, the dispatch
layer rejects it.

### Default agent

The default agent (primary) has no scope restrictions — empty lists on all
four dimensions. Scoping is for sub-agents that need constrained access.

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
