# crabtalk-skill

The [SKILL.md](https://agentskills.io) standard: the type, the format, and
discovery on disk. Nothing here knows what a runtime or an agent is.

```rust
let skill: Skill = text.parse()?;                      // the format
let all = discover::list(&roots).await?;               // what is installed
let one = discover::load(&roots, "review").await?;     // one by name
```

`allowed-tools` accepts either a YAML sequence or a comma-separated string,
because skills in the wild are written both ways.

## License

Apache-2.0
