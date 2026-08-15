# crabtalk-storage

Persistence backends for Crabtalk.

The `Storage` trait is declared in [`crabtalk-core`](../core); this crate
implements it. A backend is chosen at compile time through the `Storage`
associated type on `runtime::Config`, so a host picks one by wiring a type,
not by flipping a switch.

Split out of the daemon library so a consumer — a desktop app, a cloud
tenant — can implement `runtime::Config` against a backend without
depending on `crabtalk` itself.

## Backends

| Type             | Feature  | Notes                                        |
|------------------|----------|----------------------------------------------|
| `FsStorage`      | —        | TOML configs, markdown prompts, JSON sessions |
| `SqliteStorage`  | `sqlite` | One database file per tenant, via `sqlx`      |

`FsStorage` is what the daemon runs: agents in `local/settings.toml`,
prompts under `agents/<ulid>/prompt.md`, and one append-only directory per
session under `sessions/<ulid>/`.

`SqliteStorage` keeps sessions, agents, and the install config in a single
file, so a tenant is something you can copy, move, or delete whole. Skills
are read from the filesystem either way — they are content, not state.

The feature is about driver cost, not selection: nothing that only wants
the filesystem should build `sqlx`. Which backend a host runs is the
`Storage` associated type, so enabling a feature never changes it, and
there is no "pick exactly one" rule to trip over.

## SQL portability

The SQLite backend is written to move: no `INSERT OR REPLACE`, no
`AUTOINCREMENT`, no `PRAGMA`-dependent behaviour, and upserts spelled
`ON CONFLICT … DO UPDATE`, which PostgreSQL speaks natively. What is not
portable is placeholder syntax — `?` here, `$1` there — so a Postgres
backend is a mechanical rewrite of the bind sites rather than a redesign.
That is deliberately not abstracted: `sqlx::Any` would cost the concrete
types and the compile-time-checked macros to buy a runtime choice the
associated type already makes.

## Scaffolding

`scaffold_config_dir` lays out `~/.crabtalk/` on first run, writing
[`config.toml`](config.toml) and [`settings.toml`](settings.toml) if they
are absent. Both templates ship in this crate, since the code that writes
them lives here.

## License

MIT OR Apache-2.0
