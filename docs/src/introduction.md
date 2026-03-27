# Introduction

This is the crabtalk development book — the knowledge base you check before
building. It captures what crabtalk stands for, how the system is shaped, and
the design decisions that govern its evolution.

For user-facing documentation (installation, configuration, commands), see
[crabtalk.ai](https://crabtalk.ai).

## How this book is organized

- **[Manifesto](manifesto.md)** — What crabtalk is and what it stands for.
- **[Architecture](architecture.md)** — The system shape: crate layering,
  where features go, boundary contracts, and what the system can do today.
- **[RFCs](rfcs/README.md)** — Design decisions. Each RFC captures a specific
  problem, the decision we made, and why.

## Contributing

### Prerequisites

- Rust (stable)
- [protoc](https://grpc.io/docs/protoc-installation/) (for protobuf codegen)

### Build & test

```sh
cargo check --workspace        # compile check
cargo nextest run --workspace  # tests
cargo clippy --all -- -D warnings
cargo fmt --check
```

### Code style

- Group imports: `use foo::{Bar, Baz};` — never individual `use` lines for the same crate.
- No empty lines between `use` items. `mod` declarations go after all `use` items.
- Never use `super::` — always `crate::`.
- Each `.rs` file has a single, focused responsibility.
- Tests live in `tests/` next to `src/`, never inline `#[cfg(test)]` blocks.
- Inherit workspace dependencies — `{ workspace = true }` in member crates.
- Binary entry points go in `src/bin/main.rs`.

### Commits

[Conventional commits](https://www.conventionalcommits.org/):
`type(scope): description`. No issue numbers in commit messages — link issues in
PR descriptions.
