# Providers

Providers are the sole point of contact between the daemon and an LLM. The provider layer is external: its trait, types, and concrete implementations live upstream in `crabllm`. Crabtalk consumes providers but does not define them.

## Boundary

The `crabllm-core` crate defines the `Provider` trait and the shared types that flow across it: `ChatCompletionRequest`, `Message`, `Tool`, `ToolCall`, `Role`, `Usage`, `ApiError`. These types are the contract between crabtalk and any LLM backend.

The `crabllm-provider` crate defines concrete provider implementations. `ProviderRegistry` assembles them and yields one `Provider` value constructed from the node configuration.

Crabtalk depends on both crates as external dependencies. It does not vendor provider code. Changes to provider internals — authentication, request formatting, streaming, error decoding, retry policy — are made upstream.

## Usage

A runtime is parameterized by `Config::Provider`. The daemon's default config resolves `Provider` by calling `ProviderRegistry::build` with the user's configuration. The runtime holds a single provider instance for its lifetime and calls it once per agent step.

The provider is asked to produce:

- A non-streaming completion for synchronous operations.
- A streaming completion for `StreamMsg` operations, yielding chunks that the runtime accumulates into a `Message`.

The runtime does not interpret provider-specific errors. `ApiError` is surfaced to the client as a protocol error; the provider is responsible for mapping backend failures into `ApiError` values.

## Tools across the boundary

Tool schemas are declared in `crabllm-core::Tool`. The runtime collects schemas from the composite hook, attaches them to the request, and lets the provider format them for the backend. Tool calls returned by the provider arrive as `ToolCall` values; the runtime dispatches each call through `Env::hook().dispatch`.

The shape of tool schemas is fixed by `crabllm-core`. A tool that cannot be expressed in that shape is not expressible to crabtalk.

## Configuration

Provider configuration is read from the node's `config.toml` and passed to `ProviderRegistry`. The daemon does not inspect provider-specific configuration; it forwards the relevant sections to the registry and accepts the resulting `Provider`.

Adding a new backend is a change to `crabllm-provider`. It is not a change to crabtalk.

## Upstream

`crabllm` is maintained at [`crabtalk/crabllm`](https://github.com/crabtalk/crabllm). Bug fixes, new backends, and trait changes are filed there. Crabtalk upgrades its crabllm dependency on release.
