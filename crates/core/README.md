# crabtalk-core

Stateful agent execution library.

Provides `Agent<P: Provider>` with step/run/run_stream execution, `AgentConfig`,
`AgentBuilder`, the `ToolDispatcher` trait for tool execution, and event types
(`AgentEvent`, `AgentStep`, `AgentResponse`). Also includes the unified `model`
module with `Model<P>` wrapper, `Message`, `Tool`, `Request`, and `Response`.
The `Provider` trait comes from `crabllm-core`.

## License

MIT OR Apache-2.0
