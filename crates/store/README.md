# crabtalk-store

Persistence for Crabtalk.

Two primitives and one composite. `KVStorage` holds content, addressed by a
key the caller already has. `SqlIndex` holds only what a lookup needs to
*find* that key — ordering fields, FTS terms, set membership — and never a
body. `Store` pairs them and implements the six interfaces in `interface`
that the runtime programs against: agents, sessions, memory, skills,
harnesses, settings.

Because the index is derived, it is rebuildable by scanning a KV column, and
no write needs a transaction spanning the two: content goes in first and the
index comes out first, so a crash orphans content nothing can reach rather
than leaving a row pointing at nothing.

A backend implements the two primitives and gets everything above them for
free. `SqliteStore` is one open file behind both halves — a local install is
one file, so a realm is a thing you can copy, move, or delete whole.

## Layout

| Layer                | What implements it                          |
|----------------------|---------------------------------------------|
| `KVStorage`          | per backend — `SqliteStorage`, `MemoryDb`   |
| `SqlIndex`           | per backend — `SqliteStorage`               |
| `interface::*`       | `Store<K, Q>`, written once                 |

Keys carry a realm slot from the first byte. One realm is one store here,
so it buys nothing today; it is in the format anyway, because a backend
serving many realms is then a different `KVStorage` impl rather than a key
migration, and a read outside the realm is not expressible rather than
merely forbidden.

The `sqlite` feature is about driver cost, not selection: nothing that only
wants the in-memory store should build `sqlx`. Which backend a host runs is
the `Storage` associated type on `runtime::Config`, so enabling a feature
never changes it.

## SQL portability

The SQLite backend is written to move: no `INSERT OR REPLACE`, no
`AUTOINCREMENT`, no `PRAGMA`-dependent behaviour, and upserts spelled
`ON CONFLICT … DO UPDATE`, which PostgreSQL speaks natively. What is not
portable is placeholder syntax — `?` here, `$1` there — so a Postgres
backend is a mechanical rewrite of the bind sites rather than a redesign.
That is deliberately not abstracted: `sqlx::Any` would cost the concrete
types and the compile-time-checked macros to buy a runtime choice the
associated type already makes.

## License

MIT
