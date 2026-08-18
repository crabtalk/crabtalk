# crabtalk-store

Persistence for Crabtalk.

One primitive. A store implements `KVStorage` — `get`, `put`, `delete`,
`scan_keys`, `scan` — and is thereby already an `Agents`, a `Sessions`, a
`Memory`, a `Skills`, a `Harnesses` and a `TextSearch`: each is bounded on
`KVStorage`, carries its own method bodies, and is blanket-implemented. There
is nothing to construct and nothing to wire.

Secondary indexes are keys too, ranked full-text included, so nothing here
needs a query planner. This crate links no database and no search engine —
which store to run is the application's choice, and `apps/agent` is five
methods over `lib/crabdb`.

The keyspace and the search design are specified in
[Storage](../../docs/src/spec/storage.md); the reasoning is
[RFC 0207](../../docs/src/rfcs/0207-store.md).

## License

MIT
