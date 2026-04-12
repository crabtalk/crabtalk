# crabtalk

The Crabtalk daemon core library.

Runs as a background service, exposing agents over UDS and TCP. Manages
agent configuration, skills, MCP servers, built-in memory, and task delegation.

Used by `crabtalkd` (the daemon binary) and `crabtalk-tui` (with the `daemon`
feature) to provide the full daemon experience.

## Features

- `fs` (default) — filesystem-backed storage
- `native-tls` (default) — OS TLS stack (SecureTransport on macOS, OpenSSL on Linux)
- `rustls` — pure-Rust TLS via rustls (for cross-compilation)

## License

MIT OR Apache-2.0
