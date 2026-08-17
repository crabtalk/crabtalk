# crabtalk-proto

The Crabtalk wire protocol: one schema, one crate, and the route between a
message and a typed call.

Bare, this is `no_std` over an allocator — the generated messages and nothing
else, which is what a harness links inside the sandbox. Each feature adds one
half of the host's world, so nothing pays for a side it does not speak:

| Feature  | What it adds                                                    |
|----------|-----------------------------------------------------------------|
| `std`    | `prost/std`                                                     |
| `server` | `Server` — a `ClientMessage` in, one typed handler out           |
| `client` | `Client` — build the message, unwrap the reply                   |
| `llm`    | conversions to the LLM types the messages carry                  |

`Server::dispatch` and `Client::request` are the only two things an implementor
writes; every operation is a provided method over them, typed both ways. That is
what keeps enum matching out of the daemon and out of every client.

`proto/crabtalk.proto` is compiled by `build.rs` and the generated module is
`include!`d, so the schema is the source of truth and there is no checked-in
generated file to drift from it. Maps are generated as `BTreeMap`, because a
guest has no `HashMap` to generate into.

## License

MIT
