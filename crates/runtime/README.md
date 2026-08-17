# crabtalk-runtime

Agent runtime — agent registry, conversation management, and harness orchestration.

Exposes `Runtime<C>` (the main entry point), `Conversation` (in-memory
conversation state), and the `Env` and `Harness` traits used to extend the
runtime with tools, event sinks, and environment-specific behavior. Persistence is
delegated to the `Storage` trait from `crabtalk-core`; memory is delegated to
`crabtalk-memory` via `SharedMemory`.

## License

MIT OR Apache-2.0
