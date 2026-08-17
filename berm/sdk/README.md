# berm-sdk

Build a harness for [Crabtalk](https://github.com/crabtalk/crabtalk).

A harness is code the daemon schedules: one RV64IMAC ELF, confined to its own
address space, reaching the world only through host calls it was granted. This
crate owns the ABI, so you never see a call number, a register, or a pointer
pair.

```rust
#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

#[berm_sdk::harness(capabilities = ["log"])]
mod tools {
    use berm_sdk::{Failed, Out};

    /// Echo the query back.
    #[args(Echo)]
    pub fn echo(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        out.write(args);
        Ok(())
    }

    /// Arguments for `echo`.
    pub struct Echo {
        /// The text to echo back.
        pub query: String,
        /// Page number, zero-indexed.
        pub page: Option<u32>,
    }
}
```

Every `pub fn` is a tool and its doc comment is what the model reads when
deciding to call it. `#[args(Echo)]` derives the JSON Schema from that struct's
fields — their types, their doc comments, and `Option` for the ones the model
may omit. Nothing is deserialized for you, so a harness that wants no JSON
parser links none.

Off the guest's target this is an ordinary binary, which is why the `cfg_attr`
above is worth copying: tools then run under `cargo test` with no RISC-V
toolchain and no daemon. See `berm_sdk::test`.

## Building

```sh
rustup target add riscv64imac-unknown-none-elf
cargo build --release --target riscv64imac-unknown-none-elf
```

```toml
# .cargo/config.toml
[target.riscv64imac-unknown-none-elf]
rustflags = ["-Clink-arg=--emit-relocs"]
```

`--emit-relocs` is not optional: without it the host cannot tell which functions
have their address taken, and rejects the image. Leaving `[build] target` unset
is deliberate — it is what keeps the host build, and therefore `cargo test`,
working without a flag.

The resulting ELF is the whole artifact: one file, every platform.

## Not the client

This is not [`crabtalk-client`](https://crates.io/crates/crabtalk-client), which
connects to the daemon over a socket. Both speak the same protocol; only this
one runs inside the sandbox.

## Design

[RFC 0205 — Berm](https://crabtalk.github.io/crabtalk/rfcs/0205-berm.html).

## License

MIT
