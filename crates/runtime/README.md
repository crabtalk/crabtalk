# crabtalk-runtime

Agent runtime — agent execution, session management, and harness orchestration.

Exposes `Runtime<C>` (the main entry point), `Session` (live session state),
and the `Env` and `Harness` traits used to extend the runtime with tools,
event sinks, and environment-specific behavior.

The runtime holds interfaces, never the data behind them. Agents, memory and
skills are read through `crabtalk-store` for the run that needs them and
dropped after, so whether any of it is cached is the store's decision rather
than a field here. Live sessions are the exception: a cancellation token cannot
be persisted, so it stays in the process.

## License

MIT
