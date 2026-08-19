# crabtalk-berm

Crabtalk's side of [berm](../../berm/engine): the harness hook, the
`crabtalk.protocol.call` system harness, and `crabtalk.http.fetch`.

berm itself has no crabtalk crate in its dependency list and cannot grow one
without `src/lib.rs` here moving — which is what makes "berm is embeddable
without crabtalk" compiler-checked rather than promised.

## License

MIT
