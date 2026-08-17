# berm-os

OS tools — `bash`, `read`, `edit` — as a Crabtalk harness.

These were once dispatched to whichever client was connected, so a run with no
client had no tools at all. As a harness they run wherever the runtime does,
which is the machine that owns the files.

Paths are relative to the root the harness was granted, and nothing in this
crate checks that: the root is the argument to the `fs` and `exec` grants and is
enforced host-side, so a path that escapes comes back as an error rather than as
an invariant this file has to maintain.

Arguments are deserialized into structs rather than read off a
`serde_json::Value`. That is not a style preference — `Value` reaches the
sandbox's one unsupported construct, dynamic dispatch, and traps.

The buffer is 256 KiB rather than the SDK default: a read of two thousand lines
is the tool's own limit and has to fit in one result. The buffers live in `.bss`
and are zeroed per invocation, which against an LLM round trip is not a cost
worth trading a truncated read for.

```sh
cargo test -p berm-os        # tools run natively, no toolchain
make harness                 # build all four and install to ~/.crabtalk/harnesses
```

Grants: `fs`, `exec`, both bounded by `root`.

## License

MIT
