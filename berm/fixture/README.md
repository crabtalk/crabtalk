# berm-fixture

The reference guest — the smallest real harness, and what berm is measured and
tested against. Not published, and not useful on its own.

Everything below the `#[harness]` line is what an author actually writes; the
exports, the manifest section, the dispatch and the panic handler come from the
SDK. Each tool prices or proves exactly one thing:

| Tool     | What it is for                                              |
|----------|-------------------------------------------------------------|
| `echo`   | typed arguments across the boundary                          |
| `chatty` | 100 host calls, to price one                                 |
| `probe`  | allocates, proving the heap arrives without a second entry   |
| `boom`   | fails on purpose, exercising the failure channel             |

`berm/engine/examples/measure.rs` reads the numbers off them. The tests in
`src/bin/main.rs` are the only exercise the SDK's host-side `test::call` gets,
and they run natively — no RISC-V toolchain needed.

```sh
cargo test -p berm-fixture
cargo build -p berm-fixture --release --target riscv64imac-unknown-none-elf
```

## License

MIT
