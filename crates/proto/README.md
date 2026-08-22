# crabtalk-proto

The Crabtalk wire protocol: one schema, and the route between a message and a
typed call.

Bare, the crate is `no_std` over an allocator — the generated messages and
nothing else, which is what a harness links. Each feature adds one half of the
host's world:

| Feature  | What it adds                                            |
|----------|---------------------------------------------------------|
| `std`    | `prost/std`                                             |
| `server` | `Server` — a `ClientMessage` in, one typed handler out   |
| `client` | `Client` — build the message, unwrap the reply           |
| `llm`    | conversions to the LLM types the messages carry          |

`Server::dispatch` and `Client::request` are the only two things an implementor
writes; every operation is a provided method over them, typed both ways.

## License

Apache-2.0
