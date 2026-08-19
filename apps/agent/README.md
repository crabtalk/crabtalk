# crabtalk-agent

The backend a general Crabtalk install runs: the five `KVStorage` methods over
[`crabdb`](../../lib/crabdb), and therefore already every interface
[`crabtalk-store`](../../crates/store) defines.

Which store to use is a deployment decision and storage engines are heavy, so
the choice lives here rather than in the store crate. One file per realm.

> The binary is a placeholder — nothing in the workspace instantiates `Backend`
> yet.

## License

MIT
