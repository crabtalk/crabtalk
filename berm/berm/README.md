# berm

A sandbox for harnesses. Loads a hash-pinned RV64 ELF, compiles it once, and
instantiates it per invocation under [rvtime](https://crates.io/crates/rvtime);
nothing survives the call.

A harness reaches the world only through system harnesses it was granted, and the
grant *is* the `Linker` it is instantiated with — an ungranted call traps
because nothing is registered for it. `fs` and `exec` ship here; anything about
the host is supplied by the embedder through `Capability`.

`berm::manifest(elf)` reads what an image claims to be without compiling or
running it.

```sh
cargo run --example measure    # prices a host call, an invocation, an allocation
```

Nothing here depends on a crabtalk crate, which is what lets the sandbox leave
that repository whenever it needs to.

## Design

[RFC 0205 — Berm](https://crabtalk.github.io/crabtalk/rfcs/0205-berm.html).

## License

MIT
