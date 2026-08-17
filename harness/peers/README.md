# berm-peers

Name the other agents in the runtime — the smallest protocol harness.

One tool, one capability, one `ClientMessage`. It exists to exercise the
protocol door end to end — the grant, the decode-time allowlist, and the
redaction — with nothing else in the way.

Naming the peers is all it does. *Reaching* one is a turn spent on another
agent's behalf, which is in no group a harness can hold.

```sh
cargo test -p berm-peers
make harness
```

Grant: `protocol:read`.

## License

MIT
