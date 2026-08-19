# crabtalk-crabdb

An append-only key-value store, in one file. Better than a directory of files
— which is the bar it was written to clear, not to beat a database.

```rust
let db = crabtalk_crabdb::CrabDb::open("store.crmem")?;
db.put(0, b"agent/one", b"{}")?;
assert_eq!(db.get(0, b"agent/one")?.as_deref(), Some(&b"{}"[..]));
db.checkpoint()?;
```

One seek per lookup, ordered prefix scans, and a crash costs at most a torn
tail. Synchronous throughout: wrap it if you are on an executor.

Knows nothing about Crabtalk — the keyspace and everything above it is
[`crabtalk-store`](https://crates.io/crates/crabtalk-store).

## License

MIT
