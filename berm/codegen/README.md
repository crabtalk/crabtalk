# berm-codegen

The `#[harness]` proc macro. Re-exported by
[`berm-sdk`](https://crates.io/crates/berm-sdk) — depend on that, not on this.

```rust
#[berm_sdk::harness(capabilities = ["log"])]
mod tools {
    use berm_sdk::{Failed, Out};

    /// Echo the argument blob back.
    #[args(Echo)]
    pub fn echo(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        out.write(args);
        Ok(())
    }

    /// Arguments for `echo`.
    pub struct Echo {
        /// The text to echo back.
        pub query: String,
    }
}
```

Every `pub fn` becomes a tool; its doc comment is the description the model
reads, and a tool without one is a compile error. `#[args(Struct)]` derives the
JSON Schema from that struct's fields. The handler still receives raw bytes, so
a harness that wants no JSON parser links none.

The expansion also carries the exports, the `.bss` buffers sized by `buffer = N`,
and the `.berm.abi` manifest — which is what lets a host learn what a harness is
without running it.

## License

MIT
