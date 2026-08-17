# berm-skill

Skill discovery and loading as a Crabtalk harness.

One tool. An exact name loads that skill's instructions; anything else lists the
catalogue — every skill when the name is empty, and the ones whose name or
description mentions it otherwise. A miss therefore costs the model one extra
call rather than an error it has to recover from.

It reaches the runtime and never the machine. Where skill files live is the
daemon's business — packages install them, and the daemon walks the directories
— so this asks over the protocol rather than holding a read grant spanning the
config and home directories to find them itself.

The catalogue is not injected into the system prompt. Listing costs a tool call
when the model wants one, rather than a tax on every request that grows with the
number of skills installed.

The buffer matches the OS harness's: a skill body is prose meant for a model to
follow and is this harness's whole payload, and instructions truncated halfway
are worse than no skill at all.

```sh
cargo test -p berm-skill
make harness
```

Grant: `protocol:read`. The format itself is
[`crabtalk-skill`](../../lib/skill).

## License

MIT
