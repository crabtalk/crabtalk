# berm

A sandbox for harnesses.

Loads a hash-pinned RV64 ELF, compiles it once, and instantiates it per
invocation under [rvtime](https://crates.io/crates/rvtime): arguments are
pulled in through host calls, the result is read back out of guest memory, and
nothing survives the call. A harness that needs state between invocations
persists it through a capability, because its heap is gone by the time the next
one starts.

A harness reaches the world only through capabilities it was granted, and the
grant *is* the `Linker` it is instantiated with. An ungranted call traps because
nothing is registered for it — there is no check to write and no check to
forget. Grants take arguments rather than booleans: `root` is the argument to
`fs` and `exec`, so an under-specified declaration reaches nothing rather than
everything.

`fs` and `exec` ship here because a sandbox that cannot touch files or run
commands has little to confine. Everything else an embedder needs is its own to
supply through `Capability`, whose name is hashed to a call number exactly as
the built-ins are — so an embedder's capability is not a second class of thing,
and berm never has to learn what the host it is embedded in can do.

`berm::manifest(elf)` reads what an image claims to be — ABI version, tools,
schemas, capabilities wanted — out of the `.berm.abi` section without compiling
or running it. Listing a registry or assembling a prompt never means giving a
harness a turn.

## Not a crabtalk crate

Nothing here depends on a crabtalk crate, and it cannot grow one without
`crates/berm/src/lib.rs` moving. That split is compiler-checked rather than
promised, and it is what lets the sandbox leave that repository whenever it
needs to.

## Measuring

`cargo run --example measure` prices a host call, an invocation, and a guest
allocation against `berm-fixture`.

## Design

[RFC 0205 — Berm](https://crabtalk.github.io/crabtalk/rfcs/0205-berm.html).

## License

MIT
