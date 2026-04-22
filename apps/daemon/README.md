# crabtalkd

The Crabtalk daemon binary.

Wraps `crabtalk` (the daemon library) with CLI dispatch, system service
installation (`launchd` on macOS, `systemd` on Linux, `schtasks` on Windows),
foreground execution, plugin install/uninstall, log tailing, and dispatch to
external `crabtalk-<name>` binaries.

First-time setup walks the user through provider configuration interactively
so the daemon can come up against a working LLM endpoint.

## Features

- `native-tls` (default) — OS TLS stack (SecureTransport on macOS, OpenSSL on Linux)
- `rustls` — pure-Rust TLS via rustls (for cross-compilation)

## License

MIT OR Apache-2.0
