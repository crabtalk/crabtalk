# crabup

Installs crabtalk, and defines where a crabtalk install lives. Usually the
first thing you install, and the only one you install with cargo.

A thin wrapper over `cargo install`, plus the layout of `~/.crabtalk` —
defined here the way rustup defines `~/.rustup`, so every other crate reads
the paths from crabup rather than re-deriving them. It is not a registry:
crates.io is.

## Install

```bash
cargo install crabup
```

## Usage

```bash
crabup install                      # or `crabup update` — the same command
crabup install --version 0.0.24     # pin
crabup install --nightly            # build the development branch
crabup install --features rustls
crabup uninstall
crabup list
```

crabup installs *crabtalk*, not a binary you name. Today that is the crate
`crabtalk-agent`, which installs the daemon as `crabtalkd`; `crabtalk-cli`
joins it as `crabtalk` when it exists. They go on and come off together,
because they speak one protocol to each other and a machine holding two
versions of it is a wire mismatch. `crabup list` prints both names, since the
one you install is not the one you run.

`--nightly` builds from the `dev` branch of the repository against its
committed lockfile, rather than the release on crates.io. It takes no version:
there is one tip.

`install` and `update` are one command. `cargo install` upgrades when a newer
version exists and does nothing when it does not, which leaves a second verb
with no work.

## Where things land

Binaries go where `cargo install` puts them, `~/.cargo/bin`, and
`~/.cargo/.crates.toml` is the record `crabup list` reads. Separately,
`~/.crabtalk` is the daemon's root — its store, config, harness images and
socket — and `$CRABTALK_HOME` points that somewhere else.

## License

Apache-2.0
