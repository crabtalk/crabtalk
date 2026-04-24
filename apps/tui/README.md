# crabtalk-tui

Interactive TUI client for the Crabtalk daemon.

Provides an interactive REPL, conversation management, and provider/MCP
configuration — all communicating with the daemon over Unix domain sockets
or TCP.

## Features

- `daemon` — embeds the daemon crate so `crabtalk-tui --foreground` can run
  the daemon in-process (all-in-one mode) and `--reload` / `--events` /
  `pull` / `rm` can drive a running daemon over the socket.

The TUI never installs or starts the daemon as a service — that's `crabup`'s
job (`crabup daemon start`). Without a running daemon, the TUI exits with a
hint pointing at crabup.

## License

MIT OR Apache-2.0
