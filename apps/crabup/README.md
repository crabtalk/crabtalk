# crabup

Installer and service manager for the Crabtalk ecosystem. Usually the first
thing you install, and the only one you install with cargo.

A thin wrapper over prebuilt GitHub releases and `cargo install`, plus the
one thing neither does: `launchd` / `systemd` / `schtasks` lifecycle for
every crabtalk binary. It is not a registry — crates.io is.

## Install

```bash
cargo install crabup
```

## Usage

Binaries come in two kinds, and the verb says which, following `rustup`'s
split between toolchains and components.

**Apps** are what you run — the daemon, the CLI, chat gateways:

```bash
crabup install daemon cli    # one or more, by short name
crabup uninstall daemon
```

`install` also takes any crate name verbatim, so a third-party gateway works
without a table entry: `crabup install some-crabtalk-gateway`.

**Harness services** attach to a running daemon:

```bash
crabup add cron search
crabup remove cron
```

Naming one to the wrong verb is an error that names the right one. `add` and
`remove` take only known harness services — an arbitrary crate can't be shown
to be one, so it belongs to `install`.

Both installing verbs accept the same flags:

```bash
crabup install daemon --version v0.0.23     # pin
crabup install daemon --source              # build with cargo instead
crabup install daemon --features rustls     # implies --source
```

Everything else:

```bash
crabup list                  # what's known, installed, and running
crabup update                # bump released binaries to the latest version
```

`update` is always batch — it aligns the whole set, which is what keeps the
wire protocol consistent. To move one binary alone, install it at a pinned
version. It covers what crabup pulled from a release, which is what
`~/.crabtalk/installed.toml` records; anything built with `--source` went
through cargo and isn't tracked there, so `cargo install` owns updating it.

## Service lifecycle

Any short name is also a service namespace; crabup forwards the rest of the
line to that binary:

```bash
crabup daemon start          # install the platform unit and load it
crabup daemon stop
crabup cron logs
```

## Where things land

Prebuilt binaries go in `~/.crabtalk/bin/`, with versions tracked in
`~/.crabtalk/installed.toml`. The `cargo install` path — used by `--source`,
and as a fallback when a release download fails — puts them in
`~/.cargo/bin/` instead. Lookup tries `~/.crabtalk/bin/`, then
`~/.cargo/bin/`, then `PATH`, so either install route is found.

## License

MIT OR Apache-2.0
