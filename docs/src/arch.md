# Architecture

Crabtalk is a daemon, a library, and a sandbox. Which of the three you are
using decides what you can extend and how — and most design arguments here
turn out to be arguments about which one someone means.

## The layers

```
protocol   clients, over a socket           any language, untrusted
harness    third parties, in a sandbox      any language → RV64, declared grants
hook       embedders, in-process            Rust, total trust
runtime    lifetime orchestration           the thing that eats hooks
storage    data                             agents, sessions, config
```

They are not five ways to do the same thing. Read top to bottom, the first
three are **extension points graded by trust**, and the grading explains each
one's shape rather than merely describing it:

- A **harness** declares its grants because nobody trusts it. It reaches the
  world only through capabilities named in the agent's declaration, and the
  absence of a capability from its linker *is* the enforcement.
- A **hook** declares nothing because whoever compiles it in already owns the
  binary. `Hook` is public API at the runtime layer: an embedder implements it,
  registers it, and gets tools in their own process without a daemon.
- The **protocol** has capability groups because clients are outside the
  process entirely, and because a harness reaching back through
  `crabtalk.protocol.call` is a third party holding a client's surface.

`runtime` owns the *architecture of lifetime* — turns, conversations, the
agent loop — and nothing else. `storage` owns the data.

## Where does a thing go?

Three questions, in order. They are independent, and a feature can want more
than one answer.

**1. Does the daemon own the state?** If not, it cannot be protocol — there is
no question a client could ask that the daemon knows the answer to. Web search
is the clean example: everyone has it, but the daemon holds no search state, so
it is a harness or a hook and never a message.

**2. Who is the consumer — clients, or embedders?** The protocol makes the
*client* portable: anything on the socket gets the feature. A hook makes the
*implementation* portable: anything embedding the library gets it. A chat UI
cannot function without enumerating and searching conversations, so sessions
are on the wire. An embedder wants memory tools without running a daemon, so
memory is a hook.

**3. Should the wording be replaceable?** Anything that decides how a result is
phrased to a model is policy, and policy belongs where it can be forked. That
is the harness. The daemon answers `SearchSessions`; a harness decides what a
hit reads like.

A hook is also what you have when none of these has been decided yet. That is
not a criticism — it is where things start — but a hook that only ever serves
the daemon's own clients was probably a protocol message, and one that only
ever formats output was probably a harness.

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
harness. Refusing a wrong value would only teach the guest to send the right
one.

## berm is not a crabtalk feature

`berm/engine`, `berm/sdk` and `berm/codegen` have no crabtalk crate in their
dependency lists and cannot grow one without `crates/berm/src/lib.rs` moving.
That split is compiler-checked rather than promised, and it is why the sandbox
can leave this repository whenever it needs to.

`crates/berm` is crabtalk's *side*: the hook that surfaces harness tools, and
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
