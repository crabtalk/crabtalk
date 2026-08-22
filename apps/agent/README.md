# crabtalk-agent

The daemon a general Crabtalk install runs. Installs as `crabtalk-agent`, runs
as `crabtalkd`.

Two things in one crate. The backend is the five `KVStorage` methods over
[`crabdb`](../../lib/crabdb), and therefore already every interface
[`crabtalk-store`](../../crates/store) defines — which store to use is a
deployment decision and storage engines are heavy, so the choice lives here
rather than in the store crate. Around it is the process: it opens that store,
starts [`crabtalk`](../../crates/crabtalk) over it, binds a UDS socket and a
TCP port, and runs in the foreground until `SIGTERM`.

```bash
crabtalkd            # run it
crabtalkd --help
CRABTALK_HOME=/tmp/x crabtalkd    # a second install, sharing nothing
```

## License

MIT
