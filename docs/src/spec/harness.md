# Harnesses

A harness is code the daemon schedules: one hash-pinned RV64IMAC ELF, compiled
and run in-process, confined to its own address space, reaching the world only
through host calls it was granted. It never runs of its own accord — the daemon
decides when, and while running it may call back in.

Harnesses are how third-party code extends an agent. They are also how the
daemon stops shipping opinions: `bash`, `read` and `edit` are a harness an
agent declares, not tools every agent gets.

## The grant is the linker

Host functions are keyed by number, and a number with nothing registered traps.
So a capability the declaration did not grant is not *checked for* — it is
absent from the linker the guest was instantiated with. **Enforcement is the
absence of code.** There is no check to write and none to forget.

Two capabilities ship with the sandbox because they are about the machine,
which every host has:

| Capability | Reaches | Bounded by |
|------------|---------|------------|
| `fs` | Files | `root` |
| `exec` | Commands | `root` |

Anything about the *host* arrives as an embedder-supplied capability. Crabtalk
supplies two:

| Capability | Reaches | Bounded by |
|------------|---------|------------|
| `protocol:*` | The daemon's own API | the granted group, and the declaring agent |
| `http` | The network | `hosts` |

## The argument is the grant

Every capability takes an argument, and the argument is not optional decoration
— it *is* the grant. `root` bounds `fs` and `exec`; `hosts` bounds `http`.
Without the argument the capability is never registered, so an under-specified
declaration reaches **nothing** rather than everything.

That is also why the daemon's own port is not a side door: `http` can only
reach a name written in `hosts`, so `localhost` is unreachable unless somebody
put it there.

## Manifest, not inference

A harness carries `.berm.abi`, an ELF section holding its ABI version, tools,
requested capabilities, and `usage`. A section rather than an export, because
**learning what a harness claims to be must not mean running it** — the daemon
reads a tool list, a schema, and usage text out of the file without compiling
anything.

What the manifest asks for is documentation. What the agent's `HarnessConfig`
grants is what it gets, and the daemon never infers one from the other. If a
manifest could grant itself anything, "what can this agent reach?" would be a
question you answer by reading every image instead of one config.

## Images are content-addressed

An image is keyed by a digest of what determines it: the ELF, its `Grants`, the
`Scope` its capabilities close over, and the granted hosts. Not by the agent
that declared it.

Two agents that declare the same ELF against different roots hash differently
and get two linkers. Two that declare it identically share one image. A rename
changes nothing, because the agent's name was never part of the key — but a
per-agent narrowing *is* part of it, so two agents holding `protocol:sessions`
deliberately get two images rather than sharing one narrowing.

## Invocation

Memory is per-invocation: a fresh store each call, nothing surviving between
them. Anything a harness needs to persist belongs in a capability, not in its
heap. The boundary costs roughly 17µs; compiling an image is ~15ms cold and
~3ms against the on-disk code cache, paid per image rather than per call.

Entering a guest blocks the thread it runs on, and `exec` can hold it for the
length of a command, so dispatch hands the invocation to the blocking pool. A
watchdog bounds how long a guest may run, set to outlast the longest host call
a capability may make.
