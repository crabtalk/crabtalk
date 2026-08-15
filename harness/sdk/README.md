# crabtalk-harness-sdk

Build a harness for [Crabtalk](https://github.com/crabtalk/crabtalk).

A harness is code the daemon schedules: one RV64IMAC ELF, confined to its own
address space, reaching the world only through host calls it was granted. This
crate owns the ABI, so you never see a call number, a register, or a pointer
pair.

```rust
#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

#[crabtalk_harness_sdk::harness(capabilities = ["log"])]
mod tools {
    use crabtalk_harness_sdk::{Failed, Out};

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

Every `pub fn` in the module becomes a tool, and its doc comment is the
description the model reads when deciding whether to call it. `#[args(Echo)]`
names a struct beside it, and the JSON Schema the model fills in is derived from
that struct's fields: their types, their doc comments, and `Option` for the ones
it may omit. Nothing is deserialized for you — the handler gets the blob and
parses it however it likes, so a harness that wants no JSON parser links none.

A handler that returns `Err(Failed)` reports whatever it wrote to `out` as the
failure message, so an error can be specific without needing an allocator to say
it.

The manifest — ABI version, tools, schemas, capabilities wanted — is built at
compile time and carried in a `.crabtalk.abi` section, so a host reads what a
harness claims to be without running it.

## Testing

Off the guest's target a harness is an ordinary binary, so its tools run under
`cargo test` — no RISC-V toolchain, no daemon, no rvtime. `test::call` invokes a
tool the way the host does: same argument transfer, same buffer limits, same
failure channel.

```rust
#[cfg(test)]
mod tests {
    use crabtalk_harness_sdk::test;

    #[test]
    fn echo_wraps_the_payload() {
        let out = test::call(crate::crabtalk_tool_echo, br#"{"query":"hi"}"#).unwrap();
        assert_eq!(out, br#"{"echo":{"query":"hi"}}"#);
    }
}
```

A capability with no stand-in — `http`, say — panics naming itself rather than
returning a plausible zero, so a test that reached one says so.

Because the build target below is pinned to the guest, tests need the host
triple:

```sh
cargo test --target "$(rustc -vV | sed -n 's/^host: //p')"
```

## Building

Guests are `no_std` and compile for RISC-V. Both settings below are required —
without `--emit-relocs` the host cannot tell which functions have their address
taken, and rejects the image.

```toml
# .cargo/config.toml
[build]
target = "riscv64imac-unknown-none-elf"

[target.riscv64imac-unknown-none-elf]
rustflags = ["-Clink-arg=--emit-relocs"]
```

```sh
rustup target add riscv64imac-unknown-none-elf
cargo build --release
```

The resulting ELF is the whole artifact: one file, every platform.

## Not the client

This is not [`crabtalk-client`](https://crates.io/crates/crabtalk-client), which
connects to the daemon over a socket. Both speak the same protocol; only this
one runs inside the sandbox.

## Design

[RFC 0205 — Harness](https://crabtalk.github.io/crabtalk/rfcs/0205-harness.html).

## License

MIT
