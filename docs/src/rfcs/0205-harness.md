# 0205 - Harness

- Feature Name: Harness
- Start Date: 2026-08-15
- Discussion: TBD — related: [#197](https://github.com/crabtalk/crabtalk/issues/197), [#150](https://github.com/crabtalk/crabtalk/issues/150)
- Crates: core, harness, hooks, crabtalk, crabup
- Supersedes: [0080 (Cron)](0080-cron.md)
- Updates: [0184 (crabup)](0184-crabup.md), [0189 (Policy at the Edge)](0189-policy-at-the-edge.md), [0203 (Client-Side Orchestration)](0203-client-side-orchestration.md)

## Summary

A **harness** is code the daemon schedules: one hash-pinned RV64IMAC ELF, compiled and run in-process by [rvtime](https://github.com/crabtalk/rvtime), confined to its own address space, reaching the world only through numbered host calls it was explicitly granted. A harness never runs of its own accord — the daemon decides when it runs, and while running it may call back in.

What it calls back into is **the protocol**. `ClientMessage` is already the daemon's complete, versioned, externally-facing API, so it is the vocabulary for everything a harness does to the runtime: send to an agent, subscribe to an event, schedule a wake. One capability number carries the whole surface, with a per-harness allowlist over message types.

Harnesses replace stdio-spawned MCP servers as the way we distribute extensions. `harness/search` becomes one. `harness/cron` is deleted outright, because scheduling stops being a service and becomes the harness execution model: every harness invocation — tool call, timed wake, subscribed event — goes through one queue, and a cron entry is the case where the trigger is a time.

Two things follow that reach past extension code. Where a harness runs is a deployment property rather than a design one, which dissolves the "client tool" category: OS capabilities become a harness placed on whichever machine holds the files, and a client tool narrows to one whose result requires a human. And a harness is a third home for policy — not daemon-core, not client — which is where delegation belongs once sub-agents can reach tools without a client attached.

## Motivation

**Spawning a binary is not an extension mechanism.** Today an agent declares an MCP server (RFC 0193) and the daemon runs the named command as a child process with the daemon's own privileges: full filesystem, full network, full environment. Nothing about that grant is declared, inspectable, or revocable. The tool the model sees is `search_web`; the thing the operating system sees is an unbounded process. For code we wrote that is a manageable risk. For code a third party wrote it is the whole risk, and it is the reason there is no answer today to "can I install someone else's tool?"

**Distribution multiplies the problem.** A native binary needs a build per platform — `apps/crabup/src/github.rs:15-19` enumerates five — so a third-party tool author must produce and maintain five artifacts before anyone can install one. Most will not, and the ones who do become five chances for the artifact to differ from the source.

**The word is already in the repo, meaning two different things.** `apps/crabup/src/registry.rs:6` defines `harness/` as "the services you attach to a running system," installed with `crabup add`. Two entries carry that label. `crabtalk-search` answers requests and has no state of its own between them. `crabtalk-cron` owns a timer, wakes itself, and calls the daemon over the SDK socket. These are not the same kind of thing, and calling both a harness has cost us a definition we could reason with.

**The runtime exists and was built for this.** rvtime compiles a statically linked RV64IMAC ELF to native code with Cranelift, caches the generated code on disk keyed by a hash of the CLIF function plus ISA settings, and lets the host register numbered call-backs. Its own cache module is documented for the case where "a daemon may compile the same plugin from several processes at once." It is published as `rvtime` 0.0.1 and we own it.

## Design

### Definition

> A harness never runs of its own accord. The daemon decides when it runs; while running, it may call back in.

The distinction against a client is **initiative, not direction**. A harness calling the protocol is not a client, for the same reason a process making a syscall is not the kernel: it does not own the schedule. This is the load-bearing sentence, and it sorts the existing `harness/` directory into one of each:

- **search** — request in, response out, no lifetime of its own. A harness. It loses its process, its port, its axum server, its `mcp.rs`, and its service unit.
- **cron** — owns a timer and wakes itself. Not a harness under this definition, and not a thing we keep. What replaces it is a harness that *asks* to be woken, which is the same behaviour with the initiative inverted. See [Migration](#migration).

### The artifact

One ELF, self-describing. A sidecar manifest would mean two artifacts that can disagree, so the ELF answers for itself through required exports:

| Export | Purpose |
|--------|---------|
| `_start()` | Anchors the other exports so the linker keeps them; never called |
| `crabtalk_tool_<name>() -> (ptr, len)` | One per tool; the result of one invocation |

The manifest is not an export. It is an ELF section, `.crabtalk.abi`, carrying the same JSON — ABI version, tools, capabilities requested — read straight out of the file. **Learning what a harness claims to be must not mean running it**, and a `describe()` export meant exactly that: compile the image, map an address space, enter untrusted code, all to read a string it could have carried as data. A section is also what makes the manifest extractable by any tool with an ELF reader, so the same bytes can be published to a registry without a runtime.

**A tool is resolved by its symbol, not by an index.** rvtime looks exports up by name (`Instance::get_typed_func`), so a dispatcher inside the guest would only add an ordinal coupling between the host's tool list and the guest's declaration order, in exchange for nothing. Resolving by name also lets the host check `Instance::exports()` against what the manifest advertises and reject a harness whose description does not match its symbol table. A `TypedFunc` belongs to the module rather than to a store, so every tool resolves once at load and the handles serve every invocation after it. The prefix exists so a tool named `init` cannot collide with a reserved export.

Guest functions return at most two registers (`translator::RESULT_REGS`), which is exactly `(ptr, len)`, and take up to eight arguments (`rv::REGISTER_ARGS`). A `repr(C)` pair of `u64`s returns in `a0` and `a1` under LP64, which is how a guest hands back a buffer. Arguments travel *in* by the guest pulling them — `arg_len()`, then `arg_read(ptr, len)` — rather than the host writing into the guest's allocator, so the guest's heap stays the guest's business and there is no shared understanding of its layout to get wrong. Any capability returning variable-length data uses the same two-step. One pattern, reused.

`_start` earns its row the hard way. Nothing inside a harness image references its exports, so `--gc-sections` discards all of them and the host rejects the result as having no executable `.text`. An entry point that touches each export address keeps them, which is what rvtime's own fixtures do. It is pure ceremony from the author's point of view and therefore belongs to the SDK, not to them.

The manifest carries `abi_version`. A host that does not recognise it refuses the harness rather than dispatching into a capability the author did not mean.

### The export table is the registration

rvtime exposes `Instance::exports()`. A harness that exports `on_wake` can be scheduled; one that does not, cannot — and the same rule decides whether it is handed a heap. There is no registration call and no participation manifest — the ELF's symbol table states what it takes part in, which keeps the "one artifact" property true all the way through lifecycle rather than only for tools.

### Do not mirror `Hook`

`Hook` (`crates/runtime/src/hook.rs`) is our internal seam. `scoped_schema`, `on_build_agent`, and `scoped_tools` have the shapes they have because of how the composite hook and per-agent scoping work today. Publishing the trait as a guest ABI freezes it: every later change to `Hook` breaks every ELF in the field.

The harness ABI exposes the smallest subset that makes real harnesses possible, and grows an export only when a harness needs one. Internal hooks stay internal Rust. Concretely, the exports are `call` (a tool was invoked), `on_wake` (a scheduled instant arrived), and `on_notify` (a subscribed event fired) — three entry points, none of them a mirror of a trait method.

The hard case is events, and it is settled by not inventing anything: harnesses receive events by **subscribing through the protocol**, exactly as a client does. `AgentEvent::TextDelta(String)` is per-token and `on_event` runs inline in the streaming loop (`crates/runtime/src/engine/execution.rs:82,139`) from sync code on an async path, so a guest invocation per token would be unviable — but no exclusion rule is needed to prevent it, because the per-token stream was never on the event bus. The bus carries semantically meaningful topics like `agent:{name}:done`, which is precisely the granularity a harness can afford.

| Frequency | Surface | Treatment |
|-----------|---------|-----------|
| Rare (register, unregister, build) | lifecycle | A short blocking call is acceptable |
| Once per user message | `preprocess` | Blocking with a timeout; gated on measurement |
| Per subscribed event | `on_notify` | Enqueued, fire-and-forget; the subscription is the filter |

### Memory is per-invocation, storage is persistent

Each invocation gets a fresh `Store`: instantiate, run, drop. No state survives in guest memory. Persistence is a capability, namespaced per harness, reached through host calls.

The alternative — one long-lived `Store` per harness — is the intuitive reading of "joins the runtime's lifetime" and it is wrong. `Store<T>` is `Send` and deliberately not `Sync`, because entering a guest takes `&mut Store`. A long-lived harness is therefore a `Mutex<Store>`, and every call into it serialises: tool dispatch, events, and preprocessing, across every agent and conversation. One harness with a slow event handler becomes a global queue.

Per-invocation stores buy three things beyond that:

- **Reentrancy stops being a memory problem.** A harness calls an agent, the agent emits events, the events come back to the same harness — with a long-lived store that is a familiar class of bug; with fresh stores it is another invocation. Storage-level read-modify-write races remain, which is ordinary concurrency rather than memory corruption.
- **Upgrade is a file swap.** State was never in the ELF's address space.
- **A trap costs one invocation.** There is no corrupted long-lived heap to reason about afterwards.

The cost is measured, not assumed. A spike embedding rvtime — `crates/harness`, with a guest built by the SDK — puts a complete invocation at a **p50 of ~17µs**: `Store::new`, instantiate, both argument host calls, the guest's work, reading the result out of guest memory, and teardown. Compiling the ELF is ~15ms cold and ~3ms against the on-disk code cache, and is paid per image rather than per call.

Three things fall out of the measurements, and the third was a surprise.

Against an LLM round trip none of this registers, so per-invocation stores are affordable on the tool path, on `preprocess`, and on event triggers alike — there is no case for pooling stores or keeping them alive, and the isolation the design wants is free.

The p50 is flat from 16 MiB to 1 GiB of configured guest memory, confirming the address-space reservation is genuinely lazy: a harness that asks for room does not pay for it on every call.

**Entering the guest is the expensive part, and a host call is nearly free.** A guest entry costs ~13µs; a bare host call costs ~30ns, measured by making a hundred of them inside one invocation. That is a ratio of roughly four hundred, and it decides the shape of the ABI: *the host never enters a guest to tell it something.*

Handing over the heap is the case that proved it. The obvious design — the host enters the guest with `init(start, size)` after instantiating — doubled the cost of every invocation, and cost the same whether the region was 64 KiB or 62 MiB, because the work was never the initialization. Declaring a heap so the host could skip that entry was an improvement and still the wrong answer. The right one is that the guest *asks*: its allocator pulls the bounds through two host calls the first time something allocates, from inside the entry it is already in. A harness that never allocates never asks, so there is nothing to declare, no conditional export, and no branch in the host.

The same reasoning is why arguments are pulled rather than pushed, and it is the rule to apply to every capability added later.

Measured on one arm64 machine with a 72 KB guest, a 256-byte payload, and static buffers rather than an allocator — enough to size the boundary, not a claim about what a real harness's own work will cost.

### Capabilities

Host functions are keyed by number, and a number with nothing registered traps as `Trap::UnknownHostCall(n)`. So the grant is not a check somebody has to write and remember to enforce — **the `Linker` a harness is instantiated with is its capability set**. Enforcement is the absence of code.

Two families, and they warrant different postures:

- **Host-provided** — `log`, `clock`, `random`, `http`, `storage`. One number each; a bad grant leaks data outward. These have no protocol equivalent: per-harness persistence in particular is not a `ClientMessage`, and inventing one so that everything goes through a single door would be symmetry for its own sake.
- **The protocol** — one number, carrying the whole of `ClientMessage`. A bad grant spends the user's tokens, reads their conversations, and deletes their agents.

Two rules that do not bend:

1. **Names are permanent; numbers are derived.** `ecall` carries a number in `a7`, but we do not assign it — it is a hash of the capability's name, computed at compile time on both sides. So the contract a third party ships against is `crabtalk.http.fetch`, not `16`: adding a capability cannot collide with someone else's allocation, there is no registry of integers to maintain, and the thing we version and deprecate is a name. Solana reaches the same place by hashing syscall symbols; we get it without changing rvtime, whose linker is happy with any `u64` key.
2. **Requested is not granted.** The manifest states what a harness *wants* — documentation, enough for a client to prompt. The declaration states what it *gets*. The daemon never infers one from the other.

Every capability we write needs its own timeout. rvtime's interrupt check covers every case of non-termination in guest code — a guest can only run forever by looping, and unbounded recursion exhausts the native stack and traps — so the remaining way to wedge a worker forever is a host call of ours that never returns.

### The protocol is the syscall surface

Everything a harness does *to the runtime* — send to an agent, subscribe to an event, schedule a wake, read history — is already a `ClientMessage`. There is no reason to invent a second vocabulary for the same operations, and a second vocabulary would drift from the first.

**One capability carries the entire protocol, with a serialized `ClientMessage` as its payload.** The alternative — a capability per RPC, `crabtalk.protocol.send` and so on — is workable now that names rather than numbers are the contract, and it is still the wrong shape: `ClientMessage` is a `oneof`, so the message type is already a discriminant inside the payload. Spending a second discriminant on the ABI duplicates what protobuf carries, and buys a per-RPC SDK release before any harness can reach a newly added message. One door also means one place to enforce the allowlist, log protocol access, and rate-limit it.

The grant is therefore two-level:

- The **number** gates the family. Ungranted, it is absent from the `Linker` and traps — a harness with no protocol grant cannot reach the runtime at all, and that enforcement is still the absence of code.
- An **allowlist** gates which message types pass, checked once on decode.

Three rules govern what goes in that allowlist:

**The protocol is the vocabulary, not the grant.** `ClientMessage` is what the user's own trusted UI may do, and it includes `DeleteAgent`, `UpdateAgent`, and `Reload`. A third-party harness holding those can delete the agents it was installed to help. Default-deny, granted in groups named by intent rather than one flag per message type.

**Authority rides on the invocation, not only the declaration.** A harness invoked during agent X's tool call and calling `send` is acting as *someone*. The invocation carries `(agent, conversation, sender)`, and protocol calls are scoped to that context unless a broader grant exists. The declaration grants classes of operation; the invocation supplies the instance. It is the difference between handing a process a file descriptor and granting it permission to open any file.

**Filesystem and command capabilities are scoped, not sandboxed.** A harness granted `exec` can do anything the user can, and no amount of address-space confinement changes that. Its containment comes from the capability's own implementation — a path subtree, the per-agent bash deny rules already in `HooksConfig` — enforced host-side. Installing OS tools as a harness makes them modular and placeable; it does not make `bash` safe, and this RFC should not be read as claiming otherwise.

### Placement

Where a harness runs is a deployment property, not a design one. The same ELF, with the same protocol calls, runs in the runtime or in a client that hosts the harness runtime; only the capability implementations differ, because only the machine differs.

This dissolves the "client tool" category. The boundary was never about tenancy — RFC 0193 already settled that, with one runtime per user and multi-tenancy reached by running more of them — it was about *where the files are*:

| Deployment | Files live | Harness runs |
|---|---|---|
| Desktop | Same machine as the runtime | In the runtime; forwarding is pure overhead |
| Cloud runtime, local work | On the user's machine | In the client that hosts it |
| Cloud runtime, cloud workspace | Beside the runtime | In the runtime |

Two of three want local execution, and today's rule — `crates/hooks/src/os/mod.rs:3`, "the daemon never executes these" — was written for the one case that doesn't and is paid by the two that do.

**A client tool is one whose result requires a human to be present.** `ask_user` passes that test and stays a forwarded tool; `read` fails it and becomes a harness capability. The code has already half-found this line, special-casing `ask_user` out of the generic forward path at `crates/sdk/src/stream.rs:82`.

The reason this matters is not tidiness. `crates/crabtalk/src/bridge.rs` states the current contract: "A client's tools are exactly what it declares in `StreamMsg.tools`. There is no default set: the daemon cannot execute a client tool, so advertising one the client never claimed only buys a forward nobody answers — a hang until the timeout, not a fallback." A run with no client attached therefore has no tools at all. That is tolerable while every run has a human watching it, and this RFC ends that: a scheduled wake, an event-triggered invocation, and a harness calling `send` all produce agent runs with no client. Tools-in-client does not degrade when scheduling lands — it stops working.

Forwarding also gets the granularity wrong. A harness invocation crosses the boundary once and makes its fifty filesystem calls locally; forwarding primitives crosses fifty times. The agent still pays the one round trip it can never avoid, because the model is on the far side waiting for a result. What changes is everything else.

Two consequences worth stating. A thin client — the proto carries a `swift_prefix` option, so they are already in the picture — gains the same tools as the TUI, because tools no longer live in the client; today it cannot have `bash` by construction. And approval inverts: with execution in the runtime, prompts cross the wire instead of results, which is the rare path rather than the frequent one.

Placement has a hard ceiling worth recording: hosting harnesses requires a JIT, and iOS forbids one. Rich clients that own files can host; thin clients cannot and do not need to, because a phone is a view rather than a place to run `bash`.

### Bounding invocation chains

Making the protocol callable closes a loop that does not exist today: a harness calls `send`, an agent runs, `agent:{name}:done` publishes, the subscription wakes the harness, which calls `send`. The queue will service that forever.

Every invocation therefore carries a **chain depth**, incremented when work is enqueued as a consequence of a harness call and refused past a limit. A budget over the chain — invocations, or the tokens its agent calls spend — is the same idea with a more useful unit, and is the one place where the contract analogy is load-bearing rather than illustrative: this is gas.

Bootstrap has a floor. The queue, the due-set's storage, and protocol dispatch itself cannot be harnesses.

### Harnesses are the edge that isn't the client

RFC 0189 drew the line as *mechanism in the daemon, policy at the edge*, and "the edge" has since been read as "the client process" — because at the time it was the only other place available. What 0189 actually objected to was the daemon deciding on the user's behalf with hardcoded heuristics it could not be argued out of.

A harness is a third location. Not daemon-core, not client: installable, agent-declared, forkable, replaceable. Policy in a harness satisfies 0189 completely — the daemon still decides nothing — without requiring every client to grow its own implementation of that policy.

The principle gains a clause rather than losing one: **mechanism in the daemon, policy at the edge, and a harness is edge code that happens to run in the runtime.**

This is what keeps "the client is a thing that calls the daemon" from being in tension with "the daemon does not decide for you." A client's job reduces to rendering the stream, sending user input, answering `ask_user`, and prompting for approvals. What it must not become is the only place a capability exists, because then every client reimplements it and each client's UI constraints leak into what agents can do.

### Delegation as a harness

RFC 0203, landed the same day as this one, moved `delegate` from a daemon hook to a client tool. Its reasoning holds under its premise, and the premise is the one this RFC changes:

> "Sub-agents could think, reach memory, skills, and MCP, and nothing else. Daemon-side orchestration was orchestrating agents with no hands."

Delegation moved to the client because that is where the hands were. Once tools are harnesses in the runtime, sub-agents have hands wherever they run, and the bridge work 0203 priced out — multiplexing sub-conversation forwards, namespacing `call_id`s across conversations, propagating cancellation, keeping listener teardown from killing in-flight calls — is not paid by anyone, because nothing is forwarded.

Two compromises in 0203's design exist only because the orchestrator is a user interface. Sub-agents are not offered `ask_user`, since "the REPL's ask modal is a single slot that two concurrent sub-agents would corrupt" — a rendering constraint bounding agent capability. And they are not offered `delegate`, since withholding it "caps recursion at one level with no depth counter to maintain" — a counter this design already has, as the chain depth above.

The decisive argument is reach. Client-side orchestration means every client implements orchestration: the TUI has delegation, telegram does not, a thin client never will, and a scheduled run has no client to have it. As a harness it is installed once and inherited by all of them, including the runs with nobody attached.

What survives from 0203 is its mechanism, entirely — sub-conversations keyed by a distinct sender, each an ordinary persisted conversation, no protocol change:

```
stream(agent="reviewer", sender="delegate:{call_id}:0", tools=[…])
```

A delegation harness makes exactly those calls over the protocol from inside the runtime. 0203 proved the primitive; only the caller moves. This is a relocation, and specifically not a return to `DelegateHook`, which had no tools, no depth counter, and no forkability.

**The sequence is forced.** OS capabilities become a harness first, delegation second. Reversed, this rebuilds 0203's original complaint exactly: an orchestrator in the runtime handing out sub-agents with no hands.

### Declaration

Agents own their harnesses by value, following RFC 0193's argument for MCP and more strongly: a hash-pinned ELF is more portable than a `command` + `args` + `env` triple that assumes the destination machine already has the binary.

```toml
[[harnesses]]
name = "search"
source = "github:crabtalk/search@v0.1.0"
sha256 = "9f2a…"
capabilities = ["http", "clock"]

[[harnesses]]
name = "reminders"
source = "github:someone/reminders@v2.1.0"
sha256 = "c40e…"
capabilities = ["clock", "protocol:schedule", "protocol:send"]
```

`AgentConfig.harnesses: Vec<HarnessConfig>` sits beside `mcps`. Tools land in the agent's tool list under their own names and schemas, read from the manifest at register time — the per-agent declaration is already the gate, so there is no meta-tool indirection to pay for.

**The daemon does not download code.** crabup fetches and verifies; the daemon loads what is present and errors if it is not. A daemon that fetches third-party code because an agent config named a URL is a daemon making a policy decision with a network connection.

### Execution: one queue

Every path that can start a guest goes through one queue, with three trigger kinds:

| Trigger | Entry point | Enqueued as | Latency |
|---------|-------------|-------------|---------|
| Tool call | `call` | High priority, with a reply channel — a model is waiting | Critical |
| Due instant | `on_wake` | The scheduled case | Tolerant |
| Subscribed event | `on_notify` | Fire-and-forget; the subscription is the filter | Tolerant |

Behind all three, one executor: the blocking pool, the per-invocation timeout, the `Interrupt` handle held by a watchdog, the fresh `Store`. Guest execution blocks a thread, so it runs under `spawn_blocking`; capabilities needing async work `block_on` from inside that thread.

The queue is the security boundary as much as the scheduling one. Concurrency, timeout, per-harness rate, and single-in-flight all have exactly one place to live, and nothing starts a guest without passing through it.

### Scheduling

The due-set understands one thing: **wake harness H at instant T with payload P**. A one-shot is the base case; recurrence is a harness that re-arms itself when it wakes.

Scheduling is a protocol message, not a bespoke capability, which is what makes the due-set reachable by everyone who needs it: a harness re-arming itself, a client asking for "foo tomorrow at 08:00," or a model doing so on the user's behalf all send the same RPC and land in the same heap. The proto once carried `CreateCron` / `DeleteCron` / `ListCrons` at tags 27-29, removed when cron went standalone; the replacement is not those messages returning but a smaller one — an instant, a target, a payload — with recurrence living in the harness.

This is not a simplification for its own sake — it is strictly more expressive than teaching the host a schedule language. "Every five minutes with backoff after failure," "the third Tuesday unless it's a holiday," "hourly but not overnight" are all re-arm logic, and none of them need a host change. The `cron` crate moves into the guest SDK, where an author uses it or does not. The host never learns what a cron expression is and never acquires an opinion about DST.

The data structure is a min-heap of one-shots keyed by instant — `(when, harness, id, payload)` — owned by the host, persisted, and reloaded at startup without running a single guest. **The host never asks a guest whether it is due**; that would be an instantiation per harness per tick to answer a question the host already knows.

The payload matters because not everything that schedules is a harness scheduling itself. A person, or a model on their behalf, asks for "foo tomorrow at 08:00" — so the due-set is reachable from outside a guest call, and the wake carries enough for the harness to know which of many pending things is due.

Consequences that follow:

- **Wall-clock intent is not an instant.** "08:00 tomorrow" needs a timezone, so `clock` exposes the host's local offset, not only UTC. Because a recurring harness re-resolves local to UTC at every re-arm, DST is handled by construction — where a host-side cron parser would have to be right about it forever, in code no harness author can fix.
- **Missed occurrences are policy.** The daemon was off for three hours and thirty schedules came due. Firing all thirty is a thundering herd; dropping them silently is data loss. The wake carries both `scheduled_at` and `now`, and the harness decides — a reminder still means something two hours late, a standup post may not.
- **Overrun does not overlap.** One in-flight invocation per harness per schedule; the next occurrence skips or defers. Without it the first badly written harness pins the pool.
- **Backpressure is visible.** When a drain's due-set exceeds the worker budget, the remainder spills to the next drain and says so. A queue that grows silently is how this becomes unexplainable at 3am.
- **Re-arm before the body.** A harness that traps before re-arming silently stops being scheduled. The ABI cannot enforce ordering; the SDK's recurring wrapper re-arms first and runs the body second, so a panic costs one occurrence rather than the schedule.

### The SDK is the contract

The SDK here is `harness/sdk` — a guest library, `no_std`, compiled for `riscv64imac-unknown-none-elf`. It is **not** `crates/client` (published as `crabtalk-client`, and named `crabtalk-sdk` before this RFC), which is a std and tokio library for talking to the daemon over a socket. The two can never merge: one lives in a world with sockets and an async runtime, the other in a world with neither.

They are nonetheless the same kind of thing seen from two sides. Both are protocol clients — `crates/client` sends a `ClientMessage` over a socket, a harness sends one through a single ecall — so their surfaces should rhyme wherever they do the same work, even though they cannot share a line of transport code. Whether they can share the generated protocol types is an open question below.

Third parties should never see a call number, a `(ptr, len)`, or a register convention. They see a library: declare tools, implement the lifecycle points you care about, call typed capability wrappers. That library decides whether anyone builds a harness at all, and it is where conventions like re-arm-before-body live.

It also buys room to move: the ABI can be revised as long as the SDK absorbs it — except for ELFs already shipped, which are frozen against the capability names they were compiled with, which is what `abi_version` is for.

The SDK also builds for the host, and that is not a curiosity — it is what lets an author `cargo test` their handlers. Off the guest's target the exports are ordinary functions and the buffers ordinary memory, so `test::call` invokes a tool exactly as the host does: the same argument transfer, the same buffer limits, the same failure channel. Capabilities are served by a stand-in host a test can set; one with no stand-in panics naming itself rather than returning a plausible zero. Solana's programs work this way for the same reason, and it is the difference between finding a mishandled empty string in a second and finding it through a cross-compile and a daemon.

The SDK's first obligation is the one the spike tripped over: **generate `_start`, and make it reference every export.** Omit it and the linker discards the whole image, which surfaces as the host refusing a guest that appears, from the source, to export exactly what it should. Finding that took one failed run with rvtime's fixtures open alongside; an author without them would lose an afternoon to it. The same class of obligation covers linking with `--emit-relocs` and building for `riscv64imac-unknown-none-elf` — all of it belongs in a template and a build profile that an author never edits.

Beneath that, the SDK builds on `rvtime-guest` for the ecall wrappers rather than reimplementing them; the spike confirmed the swap is free, producing a byte-identical image. What our SDK owns is everything harness-shaped above that line: the entry anchor, the `describe`/`call` scaffolding, and typed capability wrappers.

One gap to design before the first external author hits it: **when a harness traps, its author currently gets `guest memory fault at 0x…` and nothing else.** We hold the ELF with relocations and function names in `module.program().functions`, so mapping a trap address back to a function name is available to us. A `log` capability plus symbolised traps is plausibly the difference between people building harnesses and people giving up.

### Layout

```
crates/harness    host — engine, ABI, capability table, protocol bridge, loader, queue, Hook impl
harness/sdk       guest library third parties build against
harness/*         guest crates: no_std, riscv64imac, excluded from the workspace
```

`harness/` keeps its name and sharpens its meaning: things that attach, now compiled to ELF, excluded from the workspace exactly as rvtime excludes its own `crates/guest`. The pairing follows `crates/mcp` — a `crates/` entry is our host implementation of a thing; the artifacts of that thing live elsewhere.

There is no `crates/rvtime`. rvtime is published; a crate whose content is a re-export of a published dependency is a file that exists only to drift. What we need is the embedding, and that is `crates/harness`.

### Distribution

crabup's verbs already fit — `crabup add` attaches a harness, `crabup remove` detaches it. What changes is what gets fetched:

- One `.elf` per release, no platform matrix. Apps keep theirs.
- `Entry.label` becomes `None` for harnesses — the field that already means non-serviceable. No launchd or systemd unit, because there is no process.
- The declared `sha256` is verified on fetch and on load.

### What this commits us to

Once the protocol is a syscall surface and harnesses are scheduled, confined, capability-granted units of code, the daemon is a kernel. That is a coherent thing to be and it is where this design points, but it is worth naming as a decision rather than discovering it later.

The bill: every protocol change now carries ABI weight; the queue acquires the obligations of a scheduler, including fairness and accounting; and third-party authors will expect what OS users expect — stable interfaces, resource limits they can see, and a debugging story better than a fault address. The reserved-tag discipline already visible in the proto is evidence this is a bill we can pay. It should still be paid deliberately.

## Migration

**`harness/cron` is deleted.** Its scheduler loop (`harness/cron/src/runner.rs:139-149`) already sleeps until the next occurrence inside a `KeepAlive` launchd process — there is no `StartCalendarInterval`, so the daemon has to be alive for 08:00 to fire today exactly as it would inside the runtime. Moving the mechanism inward costs one process, one service unit, and one timer task per schedule, and returns a single waiter over a sorted set. RFC 0080 is superseded; the entry leaves the crabup registry. Downstream apps that need scheduling before the harness path lands can do it with the SDK in a few dozen lines, which is the argument for not carrying a service to do it for them.

**`harness/search` becomes a guest.** It loses `mcp.rs`, its `mcp` feature, and the rmcp/axum/schemars dependencies with it. Its engines are `reqwest` and `scraper`, which are `std` and cannot cross into a `no_std` guest as they are — so search is not the first harness we ship. See [Unresolved questions](#unresolved-questions).

**`crates/hooks/src/os` becomes a harness.** The daemon stops compiling in an opinion about what a filesystem tool is and installs one instead. State the hook holds per instance moves with it: the cwd is already supplied as `req.cwd` and becomes conversation state, and the read-before-edit set — a `Mutex<HashSet<PathBuf>>` that today dies with the client and diverges across two open windows — becomes harness storage keyed by conversation. `ask_user` stays a forwarded client tool.

**`delegate` becomes a harness, after the OS one.** RFC 0203's client-side implementation stands until then; see [Delegation as a harness](#delegation-as-a-harness) for why the order is not optional. Its sender-keyed sub-conversation mechanism is unchanged by the move.

**Nothing changes for MCP.** `crates/mcp` stays as it is. A remote HTTP MCP server is someone else's process on someone else's machine and there is nothing to confine. What harnesses displace over time is the stdio case: spawning a local binary with the daemon's privileges.

## Unresolved questions

- **The capability set, and what is deliberately excluded.** `log`, `clock`, `random`, `http`, `storage`, plus the protocol is the starting shape. The exclusions need writing down alongside the inclusions, before any number is assigned.
- **How protocol grants are grouped.** Thirty-six live message types is too many to declare one by one, and "the whole protocol" is too coarse to be a grant at all. Groups named by intent (`protocol:send`, `protocol:schedule`, `protocol:read`) are the shape; which message belongs to which group, and which belong to no group a third party can hold, is the work.
- **What a real harness's own work costs.** The boundary is measured at ~12µs; the guest that produced that number has static buffers and no parser. An allocator, JSON, and actual logic are the author's cost rather than the design's, but a harness on the `preprocess` path is worth profiling before it is normal to put one there.
- **Whether the protocol types can be shared with the guest.** prost supports `no_std` with alloc, so `crates/core`'s generated types could in principle be built for a guest, giving `harness/sdk` and `crates/client` the same message structs. Whether that module is separable from the rest of `wcore` is unexamined; the fallback is generating them twice from one `.proto`, which is duplication a build script can at least keep honest.
- **Schedule granularity.** The finest interval the due-set will honour is a product decision, not a number to pick here.
- **A logical epoch counter.** Not needed for correctness with a sorted due-set, but "harness X ran at epoch N" is a coordinate that makes replay, tests, and per-epoch rate limits legible. If we want it, it should be a counter incremented per drain rather than a wall-clock heartbeat.
- **The first harness.** Something small with one capability, to exercise ABI, grants, traps, and distribution end to end without search's `no_std` port confusing the signal. Search follows; then cron, as the proof that protocol capabilities are real; then OS; then delegation, which depends on OS.
- **Whether other policy follows delegation.** RFC 0189 handed compaction timing to clients on the same reasoning that handed them delegation, and the same argument — every client reimplements it, and a clientless run has none of it — applies unchanged. Left alone deliberately until a delegation harness exists to learn from.
- **Approval.** With execution in the runtime, a capability grant that needs a human turns into a prompt crossing the wire. Where the answer is stored, and whether it is remembered per agent or per invocation, is undesigned.
- **`harness/search`'s engines.** Either the host offers HTML querying as a capability (the host keeps `scraper`) or the guest gains a `no_std` parser. This is a real rewrite and it should not ride along inside the foundational change.

## Alternatives considered

**A long-lived `Store` per harness.** The intuitive reading of "joins the runtime's lifetime." Rejected: `Store` is `Send`, not `Sync`, so it becomes a mutex that serialises every call into that harness across the whole runtime, and it re-introduces reentrancy as a memory-safety concern. Per-invocation memory with explicit storage gets the same persistence with none of it.

**Host-side cron expressions.** The host parses a schedule string and re-arms. Rejected: the host acquires a scheduling DSL and a permanent DST obligation, and the result is *less* expressive than a guest that computes its own next instant. Absolute instants plus guest re-arming is smaller and does more.

**A fixed block time.** A heartbeat that scans for due work each tick. Rejected: blockchains poll on a period because they need consensus on an ordering, and we do not have that problem. The cost is a granularity floor on every schedule — "08:00" becomes "the first tick at or after 08:00" — plus a wakeup every period whether or not anything is due. A sorted due-set with sleep-until-earliest gets the same batching, fires exactly, and idles at zero.

**Mirroring the whole `Hook` trait.** Rejected: it publishes an internal seam as a public ABI and makes every future `Hook` change a breaking change for every shipped ELF.

**One capability per protocol RPC.** The literal reading of "the protocol is the capability set," and genuinely tempting: each RPC gated by the presence of its own closure would make protocol grants enforcement-by-absence like every other capability, with no decode-time allowlist. Rejected on duplication rather than on compatibility — `ClientMessage` is a `oneof`, so the message type is already a discriminant in the payload, and putting a second one in the ABI means thirty-six registrations per harness plus an SDK release before a harness can reach any newly added message. The decode-time allowlist is one match statement in one place, and it is also where logging and rate-limiting want to live.

**Keeping OS tools and delegation client-side.** The status quo, and correct while every agent run has a human attached to it. Rejected because this RFC removes that condition: scheduled wakes, subscribed events, and harness-initiated sends all produce runs with no client, and `crates/crabtalk/src/bridge.rs` is explicit that a run with no client has no tools — an unanswered forward is "a hang until the timeout, not a fallback." The alternative also leaves each client to reimplement orchestration, which is why telegram has no delegation today.

**A bespoke event-delivery mechanism for harnesses.** An `on_event` export fed by a hand-picked subset of `AgentEvent`. Rejected once it was clear the event bus already exists and already carries the right granularity — `SubscribeEventMsg` and topics like `agent:{name}:done`. Harnesses subscribe the way clients do, and the per-token stream is excluded by never having been on the bus rather than by a filter we maintain.

**WebAssembly instead of a RISC-V ELF.** The mainstream choice, with a more mature third-party toolchain story and a component model that solves interface description properly. Chosen against because we own rvtime end to end — when a harness needs something the runtime cannot express, that is a PR to `crabtalk/rvtime` rather than an upstream negotiation — and because its numbered-`ecall` host interface is already the shape a capability grant wants. The costs are real and stated: `no_std` plus alloc only, RV64IMAC with soft float, POSIX-only hosts, and `--emit-relocs` required at link time.

**Keeping search as an in-process Rust `Hook`.** Simpler than everything above and correct for our own code. Rejected as the general answer because it does not extend to code we did not write, which is the entire problem.

## Out of scope

- **Remote MCP.** Unchanged, and not a candidate.
- **Secrets.** RFC 0193's line holds: the daemon stores literal values and whatever sits above it resolves them.
- **The guest SDK's API surface.** It deserves its own RFC once the ABI has carried a real harness.
- **Windows.** rvtime's memory and traps are POSIX; harnesses do not run there.
- **A registry protocol.** [#150](https://github.com/crabtalk/crabtalk/issues/150) covers pluggable sources; this RFC assumes crabup fetching a hash-pinned artifact from a release.
