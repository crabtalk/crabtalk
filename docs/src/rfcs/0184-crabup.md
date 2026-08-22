# 0184 - crabup

- Feature Name: crabup
- Start Date: 2026-04-24
- Discussion: [#184](https://github.com/crabtalk/crabtalk/pull/184)
- Crates: `crabup`
- Updates: [0043 (Component System)](0043-component.md)

> **Note (2026-08-22).** This RFC made service management the point: crabup owned `launchd`/`systemd`/`schtasks` lifecycle for every crabtalk binary, and `cargo install` was the part it merely wrapped. That half is gone — `crates/command` was deleted with the harness work, no platform unit is written, and there are no `start`/`stop`/`ps`/`logs` verbs. So is the resolution table: crabup installs one product and names its crates as a constant. What survives is distribution and the install layout, which is what the text below now describes. The verb history (`pull`/`rm` shipping as `install`/`uninstall`, the `add`/`remove` pair for harness services) is settled and no longer worth carrying: harnesses are ELF images with no process, per [0205](0205-berm.md).

## Summary

crabup is how crabtalk reaches a machine and stays current. It is a wrapper over `cargo install`: crates.io is the registry, workspace-version inheritance is the version coordination, and `cargo install` is already an upgrade when a newer version exists and a no-op when it is not. Prebuilt binaries are a second backend for the same verbs, and wait on a release job that does not exist yet.

It also owns the install layout. `~/.crabtalk` is defined here, the way rustup defines `~/.rustup`, and every other crate reads the paths from `crabup::dirs` instead of re-deriving them. That is the half the workspace depends on, and it is why the commands sit behind a `cmd` feature: a crate that wants `CONFIG_DIR` should not build a CLI to get it.

## Command surface

```
crabup install [--version X | --nightly] [--features a,b] [--no-default-features]
crabup update                 # the same command
crabup uninstall
crabup list
```

No target name. crabup installs *crabtalk*, which is more than one crate — today `crabtalk-agent`, whose binary is `crabtalkd`, and `crabtalk-cli`, whose binary is `crabtalk`, when it exists. Package and binary differ on purpose: the package name namespaces a registry, the binary name is what someone types. They go on and come off together, because they speak one protobuf protocol to each other and a machine holding two versions of it is a wire mismatch nobody asked for. cydonia and a berm instance join later.

## What each verb does

**`install`**, and `update`, which is an alias rather than a second command: `cargo install` each crate, passing `--version` when pinned. Installing a version already present is a no-op, and installing over an older one is the upgrade — there is nothing left for a separate verb to do.

**`--nightly`** switches the source to `--git` on the repository's `dev` branch, with `--locked` so the build matches the lockfile that branch tested against. It conflicts with `--version`: a branch has one tip, and pinning it is what `--version` against crates.io is for. This is the same pair of backends the prebuilt path will slot into — a flag choosing where the bits come from, behind unchanged verbs.

**`uninstall`** runs `cargo uninstall` for each.

**`list`** reads `~/.cargo/.crates.toml` and prints each crate with its version. There is no parallel state file: if cargo's record is wrong then cargo is wrong, and crabup being wrong with it is correct.

## Removed

**Service management.** No plist, no unit file, no supervision. The daemon runs in the foreground under whatever started it — a terminal today, a client process next. Distribution and lifecycle were coupled here because the daemon used to install itself; with the daemon out of that business, lifecycle belongs to whoever wants the process alive, not to the installer.

**Pass-through installs.** `crabup install <any-crate>` left with the table it resolved against. `cargo install` is that command.

**The GitHub download path.** Written and removed on 2026-08-22 — it fetched `{bin}-{version}-{platform}.tar.gz` from the releases page, which no job builds, so every install fell through to cargo anyway. It comes back with the release job, behind the same verbs.

**Harness images.** ELF files, not binaries with a process ([0205](0205-berm.md)), installed today by `make harness` into `~/.crabtalk/harnesses`. Something should install them; it is not this surface yet.

## Unresolved Questions

- **What `uninstall` removes.** Binary only today — `~/.crabtalk` and its store survive. That is the safe default, not a decided one.
