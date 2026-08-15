# crabtalk-harness-sdk

Build a harness for [Crabtalk](https://github.com/crabtalk/crabtalk).

A harness is code the daemon schedules: one RV64IMAC ELF, confined to its own
address space, reaching the world only through host calls it was granted. This
crate owns the ABI, so you never see a call number, a register, or a pointer
pair.

```rust
#![no_std]
#![no_main]

#[crabtalk_harness_sdk::harness(capabilities = ["log"])]
mod tools {
    use crabtalk_harness_sdk::{Failed, Out};

    /// Echo the argument blob back.
    pub fn echo(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        out.write(args);
        Ok(())
    }
}
```

Every `pub fn` in the module becomes a tool. Its doc comment is the description
the model reads when deciding whether to call it, and `#[params("…")]` carries a
JSON Schema for its arguments. A handler that returns `Err(Failed)` reports
whatever it wrote to `out` as the failure message, so an error can be specific
without needing an allocator to say it.

## Building

The guest artifact is a RISC-V build. `--emit-relocs` is not optional: without
it the host cannot tell which functions have their address taken, and rejects
the image.

```toml
# .cargo/config.toml
[target.riscv64imac-unknown-none-elf]
rustflags = ["-Clink-arg=--emit-relocs"]
```

```sh
rustup target add riscv64imac-unknown-none-elf
cargo build --release --target riscv64imac-unknown-none-elf
```

Leaving `[build] target` unset is deliberate: the same crate builds for the host,
which is what makes `cargo test` above work without a flag. Tests run constantly
and the artifact is built occasionally, so the explicit `--target` belongs on the
build.

The resulting ELF is the whole artifact: one file, every platform.

## Not the client

This is not [`crabtalk-client`](https://crates.io/crates/crabtalk-client), which
connects to the daemon over a socket. Both speak the same protocol; only this
one runs inside the sandbox.

## Design

[RFC 0205 — Harness](https://crabtalk.github.io/crabtalk/rfcs/0205-harness.html).

## License

MIT
