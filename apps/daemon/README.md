# crabtalkd

The Crabtalk daemon binary.

A thin wrapper over `crabtalk`, the daemon library: it scaffolds the config
directory on first run and starts the event loop. Everything else the daemon
does is reached over the socket rather than through this CLI.

## Usage

```bash
crabtalkd start              # install the platform service unit and start it
crabtalkd stop               # stop and uninstall it
crabtalkd run                # run in the foreground (launchd/systemd invokes this)
crabtalkd logs               # view service logs
```

Service lifecycle is normally driven through [crabup](../crabup), which
forwards the same verbs: `crabup daemon start`.

Config lives in `~/.crabtalk/config.toml`, scaffolded on first run. Set
`llm.base_url` there — the daemon warns when it is unset and serves an empty
model list.

## Administration

The daemon does not administer itself. These go over the socket, from the
client:

| Task                    | Command                                  |
|-------------------------|------------------------------------------|
| Hot-reload config       | `crabtalk reload`                        |
| Install/remove packages | `crabtalk pkg add` / `crabtalk pkg remove` |
| Agents                  | `crabtalk agent`                         |
| MCP servers             | `crabtalk mcp`                           |

Event streaming has no CLI of its own: clients subscribe over the protocol,
and `crabtalk` surfaces the stream in its console view.

## Features

- `native-tls` (default) — OS TLS stack (SecureTransport on macOS, OpenSSL on Linux)
- `rustls` — pure-Rust TLS via rustls (for cross-compilation)

## License

MIT OR Apache-2.0
