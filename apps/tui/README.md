# crabtalk-cli

Interactive REPL and CLI for the Crabtalk daemon. The binary is `crabtalk`.

Bare `crabtalk` opens the REPL. The subcommands are non-interactive admin
over the same connection — a Unix domain socket, or TCP.

## Usage

```bash
crabtalk                     # chat
crabtalk resume              # resume a previous conversation
crabtalk agent list          # create, list, delete, rename agents
crabtalk mcp list            # manage an agent's MCP servers
crabtalk pkg                 # skills and MCPs as packages
crabtalk reload              # hot-reload daemon configuration
crabtalk auth                # cloud authentication
```

Daemon lifecycle is owned by `crabup` (`crabup daemon start|stop|logs`).
Without a running daemon, the CLI exits with a hint pointing at crabup.

## License

MIT OR Apache-2.0
