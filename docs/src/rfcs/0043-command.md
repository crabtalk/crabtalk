# 0043 - Component System

- Feature Name: Component System
- Start Date: 2026-02-15
- Discussion: [#43](https://github.com/crabtalk/crabtalk/issues/43)
- Crates: command

## Summary

Crabtalk components are independent binaries that install as system services and
connect to the daemon via auto-discovery. They crash alone, swap without
restarts, and the daemon never loads them. This is the manifesto's composition
model made concrete.

## Motivation

The manifesto says: "You put what you need on your PATH. They connect as
clients. They crash alone. They swap without restarts."

This requires a system where components — search, gateways, tool servers — are
not subprocesses of the daemon. They're independent programs that run as system
services. The daemon discovers them at runtime. A broken component cannot take
the daemon down.

Other projects spawn MCP servers as child processes. If the child hangs or
crashes, it can take the daemon with it: zombie processes, leaked file
descriptors, blocked event loops. The subprocess model creates shared fate.
The component model eliminates it.

## Design

### The contract

A component is a binary that:

1. Installs itself as a system service (launchd, systemd, or schtasks).
2. Writes a port file to `~/.crabtalk/run/{name}.port` on startup.
3. Serves an HTTP API (MCP protocol) on that port.

The daemon scans `~/.crabtalk/run/*.port` at startup and discovers components
automatically. No configuration needed — drop a component on PATH, install it,
and the daemon finds it.

### Service trait

```rust
pub trait Service {
    fn name(&self) -> &str;        // "search"
    fn description(&self) -> &str; // human readable
    fn label(&self) -> &str;       // "ai.crabtalk.search"
}
```

The trait provides default `start`, `stop`, and `logs` methods:

- **start** — renders a platform-specific service template, installs and
  launches.
- **stop** — uninstalls the service and removes the port file.
- **logs** — tails `~/.crabtalk/logs/{name}.log`.

### MCP service

Components that expose tools to agents extend `McpService`:

```rust
pub trait McpService: Service {
    fn router(&self) -> axum::Router;
}
```

`run_mcp` binds a TCP listener on `127.0.0.1:0`, writes the port to the
run directory, and serves the router. The daemon discovers it on next scan.

### Platform support

Service templates are platform-specific:

- **macOS** — launchd plist (`~/Library/LaunchAgents/`)
- **Linux** — systemd user unit
- **Windows** — schtasks with XML task definition

### Auto-discovery

The daemon scans `~/.crabtalk/run/*.port` for port files not already connected.
Each file contains a port number. The daemon connects via
`http://127.0.0.1:{port}/mcp`. No subprocess management, no shared fate.

Crash? The daemon doesn't care — it was never the component's parent process.
Restart? New port file, the daemon picks it up on next reload. Update a
component? Install the new version, restart the service — the daemon sees the
new port on next scan.

### Entry point

The `run()` function handles tracing init and tokio bootstrap for all component
binaries.

## Alternatives

**Subprocess management.** The daemon spawns and manages components as child
processes. Rejected because shared fate — a broken child can break the daemon.
This is the approach we explicitly designed against.

**Docker / containerization.** Run components in containers. Rejected because
crabtalk is local-first. System services are the right abstraction for a
personal daemon on your machine.

**Shell scripts for service management.** Works on Unix, breaks on Windows,
drifts across components. A shared Rust crate is portable and stays consistent.

## Unresolved Questions

- Should the Service trait support health checks?
- Should the daemon watch the run directory for new port files instead of
  scanning only at startup/reload?
