# berm-sessions

Session search as a Crabtalk harness.

The daemon answers `SearchSessions` over the protocol, where any client can ask
it too, and this harness formats the answer into ranked excerpts — the matched
message plus surrounding context, never a whole session. It holds no storage
grant and never sees a session file: asking is narrower than reading, and the
daemon keeps the query shape, the caps, and the redaction.

Which conversations it may ask about is not this harness's decision. The
request carries an `agent` filter and the host **overwrites** it with whoever
declared the harness — refusing a wrong value would only teach the harness to
send the right one.

The 256 KiB buffer is sized for the daemon's full stretch: twenty hits, each
with a window truncated at 1 KiB. An excerpt cut in half is worse than a missing
one, because the model would cite it anyway.

`usage.md` is the few lines the model reads about *when* to reach for this —
compiled into `.berm.abi` and injected only into agents that declared the
harness.

```sh
cargo test -p berm-sessions
make harness
```

Grant: `protocol:sessions`.

## License

MIT
