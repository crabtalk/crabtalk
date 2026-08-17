# berm-codegen

The `#[harness]` proc macro. Re-exported by
[`berm-sdk`](https://crates.io/crates/berm-sdk) — depend on that, not on this.

The exports an ELF needs are ceremony: an entry point that keeps the linker
from discarding the image, a heap handshake, a manifest the host reads at
registration, and dispatch from a tool name back to a function. None of it is
the author's problem, so none of it is in their source.

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

Every `pub fn` becomes a tool and its doc comment is the description the model
reads — a tool without one is a compile error, because it is the whole basis on
which a model decides to call it. `#[args(Struct)]` names a struct beside the
tool and the JSON Schema is derived from its fields, their doc comments, and
`Option` for the ones the model may omit. The handler still receives raw bytes:
parsing is the author's choice, so a harness that wants no JSON parser links
none.

## What the expansion carries

One `#[no_mangle]` export per tool, resolved by name rather than by index — an
index would couple the two sides by declaration order for no gain. Two `.bss`
buffers sized by `buffer = N`, zeroed by the host on every instantiation. And
`.berm.abi`, the manifest built as a string at compile time, which is what lets
a host learn what a harness is without running it.

`usage_file = "usage.md"` is a path rather than the text because a proc macro
sees the unexpanded `include_str!`, never the file. The macro reads it, and the
expansion carries an `include_str!` of its own purely so cargo rebuilds when it
changes.

Off the guest's target the expansion emits a `main` instead of a `_start`, so
`cargo test` builds the same source natively.

## License

MIT
