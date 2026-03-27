# Manifesto

Crabtalk is daemon-based LLM agent infrastructure. Personal agent, local-first.

A single daemon runs on your machine, managing agents, sessions, tools, and
connections. Clients — CLI, Telegram, anything with a socket — connect and talk.
The daemon does the thinking. The client does the presenting.

## What we stand for

**Challenge first.** "Why do we need this?" comes before "How do we build this?"
If the answer is "it might be useful someday," kill it.

**Simplicity is the highest virtue.** Always choose the simplest solution that
works. Fewer abstractions, fewer indirections, fewer files. Complex abstractions
are a sign of confused thinking, not clever design.

**Abstractions must earn their keep.** Prefer plain functions over traits with
one implementor. Prefer inline logic over helpers used once. No generics, type
parameters, or layers "for future use."

**Data structures first.** If the data structures are right, the code writes
itself. If they're wrong, no amount of clever code will save it.

**Say no.** Bad ideas, overcomplicated proposals, and solutions looking for
problems get rejected. Be direct, not rude. Give the simpler alternative.

**Demand concrete examples.** "Show me the call site." "What does the error look
like?" "Walk me through the actual failure." Handwaving is not design.

**No laziness.** Find root causes. No temporary fixes, no temporary solutions.

**Minimal impact.** Changes should only touch what's necessary.

## How we build

Discuss before coding. Explore the codebase, surface trade-offs, present
options. Challenge the proposal — what problem does this actually solve? Is the
problem real? Only implement after alignment.

For bug fixes, think before fixing. Follow the bug wherever it leads. Will this
fix introduce new bugs? Does it expose a design flaw that needs a refactor? Fix
the bug, then fix what the bug revealed.
