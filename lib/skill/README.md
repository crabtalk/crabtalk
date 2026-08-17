# crabtalk-skill

The [SKILL.md](https://agentskills.io) standard — the type, the format, and
discovery on disk.

A skill is a directory holding a `SKILL.md`: YAML frontmatter naming it,
Markdown saying what it does. That is the whole format, and this crate is the
whole of it. Nothing here knows what a runtime or an agent is, which is why it
sits in `lib/`.

```rust
let skill: Skill = text.parse()?;                      // the format
let all = discover::list(&roots).await?;               // what is installed
let one = discover::load(&roots, "review").await?;     // one by name
```

`Skill` carries the fields the standard defines — name, description, license,
compatibility, `allowed-tools`, free-form `metadata` — plus the body, which is
the instructions a model follows. `allowed-tools` accepts either a YAML sequence
or a comma-separated string, because skills in the wild are written both ways.

`discover` is the rules for finding one: which roots are searched,
`check_conflicts` for the same name installed twice, and `external_roots` for
the directories outside this install. Serving a skill to an agent is not here —
that is [`berm-skill`](../../harness/skill), which asks the daemon over the
protocol.

## License

MIT
