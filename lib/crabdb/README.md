# crabtalk-crabdb

An append-only key-value store, in one file.

Better than a directory of files — which is the bar it was written to clear, not
to beat a database. A directory costs an inode per key, gives no ordered
iteration, and offers no atomicity. This packs records into one file, keeps a
resident key index so a lookup is one seek and a prefix scan is an ordered walk,
and survives a crash by discarding a torn tail.

```rust
let db = crabtalk_crabdb::CrabDb::open("store.crmem")?;
db.put(0, b"agent/one", b"{}")?;
assert_eq!(db.get(0, b"agent/one")?.as_deref(), Some(&b"{}"[..]));
db.checkpoint()?;
```

Two pieces: `format` is the CRMEM layout on disk, `CrabDb` is the store over it.
Writes append and never seek back, so a delete is a tombstone and an overwrite is
a newer record; the file is rewritten when dead records reach half the live ones
(`Options::compact_at`). `checkpoint` snapshots the key index and fsyncs, so the
next open reads instead of replaying.

The store is synchronous by design: a lookup is a seek and a read, and an async
signature would only hide that from callers who then pay for an executor they
did not need. Anything that wants it off the reactor wraps it — see
`apps/agent`, which does exactly that because compaction rewrites the file and
would otherwise stall a stream mid-response.

Knows nothing about Crabtalk. The keyspace, the columns, and everything built on
them live in [`crabtalk-store`](../../crates/store).

## License

MIT
