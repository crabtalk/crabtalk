# Architecture

Crabtalk is a daemon, a library, and a sandbox. Which of the three you are
using decides what you can extend and how — and most design arguments here
turn out to be arguments about which one someone means.

## The layers

```
protocol     clients, over a socket    any language, untrusted
harness      what shapes an agent      declared per agent, one source, two builds
capability   what an agent reaches     bounded by its argument
runtime      lifetime orchestration    turns, conversations, the agent loop
store        data                      one key-value primitive, everything above it free
```

The line worth getting right is between the middle two.

A **harness** shapes an agent: what it remembers, what skills it can load, what
it knows of its own past conversations, the tools it habitually reaches for.
Change them and it is a different agent. That is why they are declared per
agent — an agent *is* its harnesses.

A **capability** is a mechanism for reaching something that is not the agent:
`fs` and `exec` for the machine, `http` for the network, MCP for other
software. Calling another program is not shaping an agent, which is why MCP is
a capability and never a harness.

The **protocol** has capability groups because clients are outside the process
entirely, and a harness reaching back through `crabtalk.protocol.call` is
holding a client's surface. It stays exported whether or not anything shipped
here uses it — who *may* write a harness justifies it, not who does.

`runtime` owns the *architecture of lifetime* — turns, conversations, the
agent loop — and nothing else. `store` owns the data, and owns it behind a
single primitive: implement five key-value methods and every interface above
them — agents, sessions, memory, skills, harnesses, search — comes for free.

## One source, two builds

A harness is one crate compiled two ways. Built `no_std` for RV64 it is a
sandboxed ELF the daemon schedules, reaching the world only through
capabilities its agent declared — the absence of a capability from its linker
*is* the enforcement. Built with `std` it is compiled into the host and reaches
those things directly.

So compiled-in versus sandboxed is a build target, not an architecture, and
`Harness` is the one name for both. What differs is confinement: as an ELF the
declaration's grants bound it; compiled in there is no linker to omit from, and
the same grants are documentation. That follows from compiling something in
being total trust — but it is one source under two security models, which is
worth knowing before choosing a build.

`no_std` is the shared denominator rather than a floor. The compiled-in build
inherits the sandbox's constraints instead of escaping them: sync, allocating
through `alloc`. Anything that must keep state alive between invocations cannot
make the trip, because a harness gets a fresh heap every call and persists
through `fs` like anything else. MCP holds live connections, so MCP is compiled
in and only compiled in — the same test, not a second one.

## Where does a thing go?

Three questions, in order. They are independent, and a feature can want more
than one answer.

**1. Does the daemon own the state?** If not, it cannot be protocol — there is
no question a client could ask that the daemon knows the answer to. Web search
is the clean example: everyone has it, but the daemon holds no search state, so
it is a harness and never a message.

**2. Does it shape the agent, or reach past it?** What an agent remembers,
loads, and knows of its own history shapes it — that is a harness, declared by
the agents that want it. A mechanism for touching something outside is a
capability, granted and bounded by its argument.

**3. Does it hold anything alive between calls?** If yes it can only be
compiled in, because the sandboxed build gets a fresh heap per invocation.
Persisting through `fs` does not count — a file outlives the call. A live
connection does not.

## Declarations, not inference

The recurring rule, and the one worth defending hardest:

> **The declaration is the grant. The daemon never infers one from the other.**

A harness's manifest says what it *wants*; the agent's `HarnessConfig` says
what it *gets*. A manifest that could grant itself anything would make "what
can this agent reach?" a question you answer by reading every image instead of
one config file.

The same rule explains why grants take arguments rather than booleans. `root`
is the argument to `fs` and `exec`; `hosts` is the argument to `http`. Without
the argument the capability is not registered at all, so an under-specified
declaration reaches *nothing* rather than everything.

And it is why a harness never chooses its own scope: `SearchSessions` carries
an `agent` filter, and the host **overwrites** it with whoever declared the
harness. Refusing a wrong value would only teach the harness to send the right
one.

## berm is not a crabtalk feature

The sandbox lives in its own repository, [berm](https://github.com/crabtalk/berm),
and reaches this one from crates.io: `berm` host-side, `berm-lang` for guests.
Neither knows a crabtalk crate exists.

`crates/berm` is crabtalk's *side*: what surfaces sandboxed tools, and
the `crabtalk.*` capabilities. Anything host-specific belongs there. `http`
lives there rather than in the engine because hyper needs a reactor and the
engine is sync and has none — keeping the engine dep-light is keeping it
portable.

## Prose

The daemon supplies none. An agent's `description` *is* its system message,
used verbatim; there is no default prompt and no framing wrapped around it.

What a model needs in order to reach for a tool is the tool's own
`description`, or the `usage` its harness declares — a few lines about when to
reach for these tools and how they go together, which is the question no single
tool description can answer because it is about choosing between them. Usage is
declared in `.berm.abi` and injected only into agents that declared the
harness. Anything longer than a few lines is a skill, not usage.
