# crabtalk-cron

Cron scheduler for Crabtalk agents. Fires a skill into an agent on a schedule.

A client of the daemon like any other — it speaks the ordinary `StreamMsg`
protocol over the SDK, so the daemon has no cron-specific knowledge. Desktop
oriented: single-tenant, TOML-backed. Multi-tenant schedulers model their own
entry shape and storage; this is not a generic scheduling library.

## Install

```bash
crabup add cron              # fetch the binary from crates.io
crabup cron start            # install and load the service unit
```

Or drive cargo directly if you don't want crabup:

```bash
cargo install crabtalk-cron
```

## Usage

```bash
crabtalk-cron start          # install and start the service
crabtalk-cron stop           # stop and uninstall it
crabtalk-cron run            # run in the foreground (launchd/systemd invokes this)
crabtalk-cron logs           # view service logs
```

## Configuration

Admin is direct edits to `$CRABTALK_HOME/config/crons.toml`. The running
scheduler polls the file's mtime and reconciles its timers on change, so
there is no reload command.

```toml
[[cron]]
id = 1
schedule = "0 0 9 * * *"     # sec min hour day month weekday
skill = "standup"
agent = "crab"
sender = "cron"
```

`skill` is fired at `agent` as `/{skill}`, attributed to `sender`.

Optional per-entry fields:

- `once` — delete the entry once it has fired. The delete is unconditional:
  a firing that errors still consumes the entry.
- `quiet_start` / `quiet_end` — `"HH:MM"` local-time window during which
  firings are skipped. A window that wraps midnight (`"22:00"` → `"07:00"`)
  is honored. A skipped firing does not consume a `once` entry.

An entry whose `schedule` doesn't parse is logged and skipped; the rest of
the file still loads.

## License

MIT OR Apache-2.0
