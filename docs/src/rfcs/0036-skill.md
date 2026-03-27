# 0036 - Skill Loading

- Feature Name: Skill Loading
- Start Date: 2026-03-27
- Discussion: [#36](https://github.com/crabtalk/crabtalk/issues/36)
- Crates: runtime

## Summary

How crabtalk discovers, loads, dispatches, hot-reloads, and scopes skills. The
skill format follows the [agentskills.io](https://agentskills.io) convention —
this RFC covers the loading mechanism, not the format.

## Motivation

Agents need extensible behavior without recompilation. Skills are the simplest
unit that works: a markdown file with a name, description, and a prompt body.
No code generation, no plugin API, no runtime linking.

The format is defined by [agentskills.io](https://agentskills.io). What crabtalk
needs to decide is how skills are found on disk, how they're resolved at
runtime, how they stay current without restarts, and how agents are restricted
to subsets of available skills.

## Design

### Format

SKILL.md follows the [agentskills.io](https://agentskills.io) convention.
Required fields: `name`, `description`. Optional: `allowed-tools`. The markdown
body is the skill prompt.

### Discovery

`SkillHandler::load(dirs)` scans a list of directories (in config-defined order)
recursively for `SKILL.md` files. Each skill lives in its own directory:

```
skills/
  check-feeds/
    SKILL.md
  summarize/
    SKILL.md
```

Nested organization is supported (`skills/category/my-skill/SKILL.md`). Hidden
directories (`.`-prefixed) are skipped. Duplicate names across directories are
detected and skipped with a warning — first-loaded wins, in config-defined
directory order.

### Registry

A `Vec<Skill>` wrapped in `Mutex` inside `SkillHandler`. Linear scan — the
registry is small enough that indexing is unnecessary. Supports `add`, `upsert`
(replace by name), `contains`, and `skills` (list all).

### Dispatch

Exposed as a tool the agent can call. Input: `{ name: string }`.

Resolution order:

1. **Scope check** — if the agent has a skill scope and the name is not in it,
   reject.
2. **Path traversal guard** — reject names containing `..`, `/`, or `\`.
3. **Exact load from disk** — for each skill directory, check
   `{dir}/{name}/SKILL.md`. If found, parse it, upsert into the registry,
   return the body.
4. **Fuzzy fallback** — if no exact match, substring search the registry by
   name and description. If input is empty, list all available skills
   (respecting scope).

### Hot reload

The upsert on exact load (step 3) is the hot-reload mechanism. When a skill is
invoked, it's always loaded fresh from disk and upserted into the registry.
Skills can be updated on disk and picked up on next invocation without daemon
restart.

### Slash command resolution

Before a message reaches the agent, `preprocess` resolves leading `/skill-name`
commands. For each skill directory, it checks `{dir}/{name}/SKILL.md`. If found,
the skill body is wrapped in a `<skill>` tag and injected into the message. This
happens before tool dispatch — it's prompt injection, not a tool call.

### Scoping

Agents can be restricted to a subset of skills via `AgentScope.skills`. If
non-empty, only listed skills are available. Empty means unrestricted. Scoping
applies to both exact load, fuzzy listing, and slash resolution.

## Alternatives

**Code-based plugins (dylib / WASM).** Far more powerful but far more complex.
Skills are prompt injection, not code execution. The simplicity of markdown
files is the point.

**Database-backed registry.** Adds persistence complexity for a registry that
rebuilds in milliseconds from disk. Not needed.

## Unresolved Questions

- Should skills support arguments beyond the skill name (parameterized prompts)?
- Should `allowed-tools` be enforced at the runtime level? Currently it is not
  enforced — it exists in the format but has no runtime effect.
