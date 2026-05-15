# crabtalk-tui

Interactive TUI client for the Crabtalk daemon.

Provides an interactive REPL, conversation management, and provider/MCP
configuration — all communicating with the daemon over Unix domain sockets
or TCP.

Daemon lifecycle is owned by `crabup` (`crabup daemon start|stop|restart`).
Without a running daemon, the TUI exits with a hint pointing at crabup.

## License

MIT OR Apache-2.0
