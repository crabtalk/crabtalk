# 0185 - crabup

- Feature Name: crabup
- Start Date: 2026-04-24
- Discussion: TBD
- Crates: new `crabup` binary; consumes `command`; shrinks `crabtalkd`
- Updates: [0043 (Component System)](0043-component.md)

## Summary

`crabup` is a thin wrapper over `cargo install` that also owns `launchd`/`systemd`/`schtasks` lifecycle for every crabtalk binary. `crabup install crabtalkd` spawns `cargo install crabtalkd`. The value add is service management — the one thing `cargo install` doesn't do — not distribution, not version coordination, not a registry.

## Motivation

Two real problems today, both about service management, not about distribution:

1. **The daemon is its own installer.** `crabtalkd start` generates and loads a platform unit for itself via the `command` crate; every other binary (`crabtalk-telegram`, `crabtalk-wechat`, …) does the same thing with the same code. A daemon shouldn't install itself, and the install path shouldn't live in three places.
2. **No one-stop service surface.** `ps`, `logs`, `start`, `stop` are duplicated per binary and absent for most. Users need a single tool that knows about all crabtalk services on the machine, not one subcommand per binary.

Distribution is already handled: every crabtalk crate publishes to crates.io with `version.workspace = true`, so `cargo install crabtalkd` is the install story today. It will remain the install story under crabup — crabup just renames the command and wraps service management around it.

RFC [0043](0043-component.md) defined *how* components talk to the daemon (port-file discovery, MCP contract). This RFC defines *how they get installed and stay alive*.

## Design

### Command surface

```
crabup pull <name> [--version X]      # cargo install crabtalk-<name> (or crabtalkd)
crabup rm <name>                      # cargo uninstall
crabup update                         # bump every installed crabtalk-* crate to latest
crabup list                           # installed crabtalk-* crates
crabup ps                             # all crabtalk services, one view

crabup <name> start                   # install + load platform unit
crabup <name> stop
crabup <name> restart
crabup <name> logs [-f]
```

`<name>` is a short name from the resolution table below, so `crabup daemon start`, `crabup telegram start`, `crabup search logs -f`. Each short name is both a pull/rm target and a service-command namespace. `pull`/`rm` mirror the existing `crabtalkd pull`/`rm` verbs for runtime plugins so the vocabulary is consistent across the user's two install surfaces (binary-level and runtime-plugin-level).

`crabup update` is always batch — it bumps every installed `crabtalk-*` crate to the latest version on crates.io, same shape as `rustup update` over its components. There is no per-component update verb: if you only want to change one crate, that's `crabup pull <name> --version <X>`. This makes "keep the set aligned" the default behavior of the only tool users will reach for when they want newer bits, without needing atomic-set machinery to enforce it.

That's it. No `pin`, no `doctor`, no `component add` vs `pull` split — `cargo install` already handles versions; a component is just a crate you can run as a service. No atomic-set enforcement; if a user mixes versions and breaks the wire, the fix is `crabup pull <name> --version <matching>` for the mismatched one or `crabup update` to bump everything.

### `pull` is a pass-through

```
crabup pull <name>
  ↓ resolve name → crate ("tui" → "crabtalk-tui"; "daemon" → "crabtalkd")
  ↓ cargo install <crate> [--version X]
```

Name resolution is a small table compiled into crabup:

| Short name | crates.io crate     | Role                 |
|------------|---------------------|----------------------|
| `daemon`   | `crabtalkd`         | daemon               |
| `tui`      | `crabtalk-tui`      | REPL client          |
| `telegram` | `crabtalk-telegram` | Telegram gateway     |
| `wechat`   | `crabtalk-wechat`   | WeChat gateway       |
| `search`   | `crabtalk-search`   | meta-search plugin   |
| `outlook`  | `crabtalk-outlook`  | Outlook plugin       |

`crabup pull <short>` resolves via the table; `crabup pull <anything-else>` passes through verbatim so `crabup pull some-third-party-crabtalk-gateway` still works without a table edit. New first-party binaries get a row added when they ship.

Binaries land in `~/.cargo/bin`, where `cargo install` has always put them. `crabup list` reads `~/.cargo/.crates.toml` and filters for `crabtalk*`. There is no parallel state file; if `.crates.toml` is wrong, `cargo` is wrong, and crabup being wrong with it is the correct behavior.

Prerequisite: `cargo` on `PATH`. If missing, crabup prints one line pointing at `https://rustup.rs` and exits. No auto-install, no curl-pipe — the daemon doing that was part of what motivated this RFC.

### Service management (the real content)

The `command` crate already renders `launchd.plist`, `systemd.service`, and `schtasks.xml` and exposes install/uninstall/log-tail helpers. It stays. What changes is the caller: today each binary calls `command::install` from its own CLI; after this RFC only `crabup` calls into `command`. `crabtalkd start/stop/ps/logs` are deleted; so are the mirrored flags in `crabtalk-tui`.

`crabup <name> start` is:

1. Find the binary on `PATH` (fail fast if not installed).
2. Ask the binary for its service metadata — name, label, description — via a convention (`--service-info` prints TOML) or via a `Service` trait impl linked into a small `crabtalk-service` helper crate each gateway depends on. Pick one when we implement; the RFC doesn't need to.
3. Render the platform unit via `command` and load it.

`crabup ps` is the one piece that needs more than wrapping: it scans `~/.crabtalk/run/*.port` (the same directory RFC 0043 already defines) and checks each listener, then cross-references with whatever the platform's service manager reports for `ai.crabtalk.*` labels. One view, all services.

### Component model

RFC 0043 stands unchanged. A component is a binary that writes a port file on startup and serves MCP on that port. crabup doesn't alter the contract — it just installs and service-manages those binaries the same way it does `crabtalkd`. "Install a component" and "install the daemon" are the same operation under different names.

### crabllm as a managed service (optional, motivated)

Today `crabllm-provider` is a library linked into `crabtalkd`. Making `crabllm` a separate service is worth doing only if at least one of these is concrete:

- One set of provider credentials serves multiple daemons on the same machine.
- Central place for provider fallback, rate-limit smoothing, or caching.
- Swap models or provider SDKs without restarting `crabtalkd`.

None of those are pressing yet. When one is, `crabtalk-llmd` becomes another crate crabup installs and service-manages, same as any gateway. The RFC doesn't need to anticipate it.

## Impact on `crabtalkd`

| Removed from `crabtalkd` | Replaced by |
|---|---|
| `Command::Start { force }` | `crabup daemon start` (first install: `crabup pull daemon`) |
| `Command::Stop`, `Restart` | `crabup daemon stop` / `crabup daemon restart` |
| `Command::Ps` | `crabup ps` (all services, one place) |
| `Command::Logs` | `crabup daemon logs` |
| `ensure_config` + `attach::setup_llm` on first start | `crabup daemon start` first-run flow |
| Duplicate forwarding in TUI (`--start`, `--stop`) | Removed |

After this, `crabtalkd`'s CLI is `--foreground`, `reload`, `events`, and the runtime plugin ops (`pull`/`rm`, which are live-daemon operations, not install).

## Alternatives

**Plain `cargo install`, no crabup.** Installs are one command, but users hand-write `launchd`/`systemd` units per binary, and `ps`/`logs` across services don't exist. The service-management gap is the whole reason crabup is a separate tool.

**A real package manager with its own manifest, signed pre-built binaries, version coordination, atomic-set installs.** Previously drafted; cut. Infrastructure we don't need — crates.io is the registry, workspace-version inheritance is the coordination, and the non-developer audience that would need pre-built binaries doesn't exist yet. If that audience materializes, pre-built becomes a second `crabup pull` backend alongside the `cargo install` path.

**Keep each binary's `start/stop/logs` subcommand, just delete the cross-binary dispatcher.** Leaves three copies of the same install code and no one-stop service view. Cuts nothing meaningful.

**Dynamic plugin loading (shared objects).** Rejected by RFC 0043 — shared fate with the daemon is the exact thing the component model avoids.

## Unresolved Questions

- **Service metadata convention.** `--service-info` printing TOML vs. a linked `Service` trait. Either works; pick when implementing.
- **Windows service layer.** `schtasks` is weaker than `launchd`/`systemd` (no restart-on-failure, limited log routing). Acceptable for v1, or not?
- **`rm` scope.** Should `crabup rm daemon` also remove `~/.crabtalk/config/`? Leaning no (`rm` is binary-only; data stays); confirm.
- **Multiple daemon instances.** If two `crabtalkd` instances run on one machine, what owns `~/.crabtalk/`? Out of scope for v1.
