# crabtalk-mcp

An MCP (Model Context Protocol) client and bridge.

Three layers, each usable without the one above it:

- `client` — a minimal JSON-RPC 2.0 client over stdio, HTTP, or SSE
- `bridge` — a fleet of connected peers, a tool cache, and call routing
- `handler` / `dispatch` — config-driven load, port-file discovery, meta-tool
  dispatch

MCP is a *capability*, never a harness: calling another program is not shaping
an agent. It is also the clean case for compiled-in only — a peer is a live
connection, and a sandboxed harness gets a fresh heap every invocation, so
nothing that must stay alive between calls can make the trip.

## Features

Pick exactly one HTTP backend and one TLS backend; the wrong combination is a
`compile_error!` rather than a link failure.

| | Options |
|---|---|
| HTTP | `hyper` (default), `reqwest` |
| TLS  | `native-tls` (default), `rustls` |

The `hyper` backend is the more compact one — it skips reqwest's cookie store,
redirect logic, and decoders that MCP never uses.

## License

MIT
