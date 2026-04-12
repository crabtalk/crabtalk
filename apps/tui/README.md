# crabtalk-tui

Interactive TUI client for the Crabtalk daemon.

Provides an interactive REPL, conversation management, and provider/MCP
configuration — all communicating with the daemon over Unix domain sockets
or TCP.

## Features

- `daemon` — embeds daemon lifecycle commands (start/stop/foreground, plugin
  management, admin). With this feature, the TUI auto-starts the daemon and
  works as an all-in-one binary.

Without the `daemon` feature, the TUI is a pure client that requires a
running daemon (`crabtalk start` or `crabtalk foreground`).

## License

MIT OR Apache-2.0
