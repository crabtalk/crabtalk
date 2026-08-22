# Harnesses

A harness is code the daemon schedules: one hash-pinned RV64IMAC ELF, compiled
and run in-process, confined to its own address space, reaching the world only
through host calls it was granted. It never runs of its own accord — the daemon
decides when, and while running it may call back in.

A harness is what shapes an agent — what it remembers, what it can load, what
it knows of its own past, the tools it habitually reaches for. An agent *is*
its harnesses, which is why each one is declared per agent rather than shipped
to everybody.

This chapter describes the sandboxed build. The same source compiled with `std`
is the same harness reaching the host directly; see
[Architecture](../arch.md).

## The argument is the grant

Host functions are keyed by number, and a number with nothing registered traps.
So a capability the declaration did not bound is not *checked for* — it is
absent from the linker the harness was instantiated with. **Enforcement is the
absence of code.** There is no check to write and none to forget.

Every capability takes an argument, and the argument is not optional decoration
— it *is* the grant. Without the argument the capability is never registered,
so an under-specified declaration reaches **nothing** rather than everything.

| Capability | Reaches | Bounded by |
|------------|---------|------------|
| `fs` | Files | `root` |
| `exec` | Commands | `root` |
| `http` | The network | `hosts` |
| `protocol` | The daemon's own API | the declaring agent |

That is also why the daemon's own port is not a side door: `http` can only
reach a name written in `hosts`, so `localhost` is unreachable unless somebody
put it there.

## The image is the grant

There is no list of permitted capabilities beside those arguments. A published
harness is a fixed ELF: one that never calls a capability does not need to be
stopped from calling it, and one that does is a harness for exactly that. To
run a shell-less environment, install an image with no shell tool — a tool that
is absent cannot be called, where a tool present but starved of its host call
is only a broken tool.

## Manifest, not inference

A harness carries `.berm.abi`, an ELF section holding its ABI version, tools,
and `usage`. A section rather than an export, because **learning what a harness
claims to be must not mean running it** — the daemon reads a tool list, a
schema, and usage text out of the file without compiling anything.

## Images are content-addressed

An image is keyed by a digest of what determines it: the ELF, the `root` and
`hosts` bounding it, and the `Scope` its protocol door closes over. Not by the
agent that declared it.

Two agents that declare the same ELF against different roots hash differently
and get two linkers. Two that declare it identically share one image. A rename
changes nothing, because the agent's name was never part of the key — but a
per-agent narrowing *is* part of it, so two agents holding the same session
harness deliberately get two images rather than sharing one narrowing.

## Invocation

Memory is per-invocation: a fresh store each call, nothing surviving between
them. Anything a harness needs to persist belongs in a capability, not in its
heap. The boundary costs roughly 17µs; compiling an image is ~15ms cold and
~3ms against the on-disk code cache, paid per image rather than per call.

Entering a harness blocks the thread it runs on, and `exec` can hold it for the
length of a command, so dispatch hands the invocation to the blocking pool. A
watchdog bounds how long a harness may run, set to outlast the longest host call
a capability may make.
