# crabtalk-berm

Crabtalk's side of [berm](../../berm/engine).

berm knows how to run a harness and how to bound one to a directory. It does not
know what an agent is, what a `ClientMessage` is, or that a daemon exists — and
it must not, because the same sandbox runs elsewhere. This crate is everything
that knowledge lives in:

- `HarnessHook`, which loads an image, surfaces its tools to the runtime, and
  dispatches calls to them
- `crabtalk.protocol.call`, the capability that lets a harness ask the daemon a
  question — a `berm::Capability` like any other an embedder supplies, with a
  decode-time allowlist per grant group and the agent scope overwritten host-side
- `crabtalk.http.fetch`, which is here for a second reason as well: hyper needs
  a reactor, and the sandbox is sync and has none

The split is what makes "berm is embeddable without crabtalk" a fact the
compiler checks rather than a promise. berm's dependency list has no crabtalk
crate in it and cannot grow one without `src/lib.rs` here moving.

Images live in `~/.crabtalk/harnesses/`, one `{name}.elf` each. That path is
declared here rather than with the rest of the install layout because this crate
is the only thing that loads one.

## License

MIT
