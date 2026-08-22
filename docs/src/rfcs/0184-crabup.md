# 0184 - crabup

- Feature Name: crabup
- Start Date: 2026-04-24
- Discussion: [#184](https://github.com/crabtalk/crabtalk/pull/184)
- Crates: `crabup`
- Updates: [0043 (Component System)](0043-component.md)

> **Note (2026-08-22).** This RFC made service management the point: crabup owned `launchd`/`systemd`/`schtasks` lifecycle for every crabtalk binary, and `cargo install` was the part it merely wrapped. That half is gone — `crates/command` was deleted with the harness work, no platform unit is written, and there are no `start`/`stop`/`ps`/`logs` verbs. So is the resolution table: crabup manages one binary and names it as a constant. What survives is distribution and the install layout, which is what the text below now describes. The verb history (`pull`/`rm` shipping as `install`/`uninstall`, the `add`/`remove` pair for harness services) is settled and no longer worth carrying: harnesses are ELF images with no process, per [0205](0205-berm.md).

## Summary

crabup is how a crabtalk binary reaches a machine and stays current. It downloads a prebuilt release from GitHub, places it in `~/.crabtalk/bin`, and records what it installed so `crabup update` knows whether a newer version exists. `cargo install` is the fallback — when no release serves the platform, and when the flags ask for a build.

It also owns the install layout. `~/.crabtalk` is defined here, the way rustup defines `~/.rustup`, and every other crate reads the paths from `crabup::dirs` instead of re-deriving them. That is the half the workspace depends on, and it is why the commands sit behind a `cmd` feature: a crate that wants `CONFIG_DIR` should not build an HTTP client and a tar decoder to get it.

## Command surface

```
crabup install [--version X] [--source] [--features a,b] [--no-default-features]
crabup uninstall
crabup update
crabup list
```

No target name. crabup manages `crabtalk-agent`; cydonia and a berm instance are the two that will join it, and a name argument returns when the second one arrives. Until then a table of one is a constant.

## What each verb does

**`install`** resolves the latest release tag (or takes `--version`), downloads `crabtalk-agent-{version}-{platform}.tar.gz` from the releases page, extracts the binary into `~/.crabtalk/bin`, and records the version in `~/.crabtalk/installed.toml`. A failed download falls back to `cargo install crabtalk-agent`, as does any flag cargo has to honour — `--source`, `--features`, `--no-default-features`.

**`uninstall`** removes the managed binary and its manifest entry. If the binary on the machine is one cargo placed, it runs `cargo uninstall` instead.

**`update`** compares the recorded version against the latest release tag and reinstalls when they differ.

**`list`** prints the binary, whether it is installed and by which path, and the recorded version.

## Removed

**Service management.** No plist, no unit file, no supervision. The daemon runs in the foreground under whatever started it — a terminal today, a client process next. Distribution and lifecycle were coupled here because the daemon used to install itself; with the daemon out of that business, lifecycle belongs to whoever wants the process alive, not to the installer.

**Pass-through installs.** `crabup install <any-crate>` left with the table it resolved against. `cargo install` is that command.

**Harness images.** ELF files, not binaries with a process ([0205](0205-berm.md)), installed today by `make harness` into `~/.crabtalk/harnesses`. Something should install them; it is not this surface yet.

## Unresolved Questions

- **What `uninstall` removes.** Binary only today — `~/.crabtalk` and its store survive. That is the safe default, not a decided one.
