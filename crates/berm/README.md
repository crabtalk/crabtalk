# crabtalk-berm

Crabtalk's side of [berm](../../berm/berm): the harness hook and every system
harness Crabtalk serves — `crabtalk.fs`, `crabtalk.exec`, `crabtalk.http.fetch`
and `crabtalk.protocol.call`. berm serves none of its own.

berm itself has no crabtalk crate in its dependency list and cannot grow one
without `src/lib.rs` here moving — which is what makes "berm is embeddable
without crabtalk" compiler-checked rather than promised.

## License

MIT
