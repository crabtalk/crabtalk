# crabtalk-cli

The Crabtalk command line. Installs as `crabtalk-cli`, runs as `crabtalk`.

A client of [`crabtalkd`](../agent) and nothing else: it opens the daemon's
socket under `$CRABTALK_HOME`, calls one RPC, and prints the answer. Every
command is a method on `proto::api::Client`, so this crate holds formatting
and argument parsing, and no protocol.

```bash
crabtalk version           # client and daemon, so a skew is visible
crabtalk info              # uptime, agents, conversations, active model
crabtalk ps [-a]           # live conversations; -a includes stored ones
crabtalk logs <handle>     # a conversation's messages
crabtalk logs -f           # the daemon's events as they happen
crabtalk agent ls
crabtalk agent inspect <name>
crabtalk model ls
crabtalk mcp ls
crabtalk skill ls
```

There is no `run`. A turn is started by a connected agent client — this is
for looking at what the daemon holds, not for talking to it.

The daemon is not started on your behalf; nothing supervises it. When the
socket isn't there, the error says so and names the binary to run.

## License

Apache-2.0
