# crabtalk-agent

The backend a general Crabtalk install runs.

[`crabtalk-store`](../../crates/store) defines the keyspace and everything built
on it — agents, sessions, memory, skills, harnesses, and search across them —
against five key-value methods. This crate is those five methods over
[`crabdb`](../../lib/crabdb), and therefore already an `Agents`, a `Sessions`, a
`Memory`, a `Skills`, a `Harnesses` and a `TextSearch`.

Which store to use is a deployment decision and storage engines are heavy, so
the choice lives here rather than in the store crate: a runtime crate has no
business linking one. One file per realm, so a realm is a thing you can copy,
move, or delete whole.

Every method hands the work to a blocking thread. The store is synchronous — a
lookup is a seek and a read, back in microseconds — but compaction rewrites the
file, and running that on an executor thread would stall every other task in the
daemon, including a stream mid-response.

> The binary is a placeholder. What is real today is `Backend`, asserted at
> compile time to satisfy `store::Backend`, because nothing in the workspace
> instantiates it yet.

## License

MIT
