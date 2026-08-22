# crabtalk-mcp

An MCP (Model Context Protocol) client and bridge: a JSON-RPC 2.0 client over
stdio, HTTP or SSE, a fleet of connected peers with a tool cache, and
config-driven load with port-file discovery.

## Features

Pick exactly one HTTP backend and one TLS backend; the wrong combination is a
`compile_error!` rather than a link failure.

| | Options |
|---|---|
| HTTP | `hyper` (default), `reqwest` |
| TLS  | `native-tls` (default), `rustls` |

`hyper` is the more compact one — it skips reqwest's cookie store, redirect
logic, and decoders that MCP never uses.

## License

Apache-2.0
