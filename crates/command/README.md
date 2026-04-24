# crabtalk-command

High-level service command layer for Crabtalk.

Provides the `#[command]` proc-macro attribute, `Service` trait, and shared
runtime entry point (`run`) for building Crabtalk service binaries. Services
get automatic `start`, `stop`, `run`, and `logs` subcommands via generated
clap enums. Supports MCP and client service kinds.

## License

MIT OR Apache-2.0
