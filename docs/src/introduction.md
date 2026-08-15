# Introduction

This is the crabtalk development book — the knowledge base you check before
building. It captures what crabtalk stands for, how the system is shaped, and
the design decisions that govern its evolution.

For user-facing documentation (installation, configuration, commands), see
[crabtalk.ai](https://crabtalk.ai).

## How this book is organized

- **[Manifesto](manifesto.md)** — What crabtalk is and what it stands for.
- **[The workspace](arch.html)** — Every crate and app drawn to scale, the dependency graph between them, and one message traced end to end.
- **RFCs** — Design decisions and features.

## RFCs

Code tells you *what* the system does. Git history tells you *when* it changed.
RFCs tell you *why* — the problem, the alternatives considered, and the
reasoning behind the choice. When you're about to build something new, RFCs are
where you check whether the problem has been thought through before.

Not every change needs an RFC. Bug fixes, refactors, and small improvements go
through normal pull requests. RFCs are for decisions that establish rules,
contracts, or interfaces that others need to know about before building.

### Format

Each RFC is a markdown file with the following structure:

- **Header** — Feature name, start date, link to discussion, affected crates.
- **Summary** — One paragraph describing the decision.
- **Motivation** — What problem does this solve? What use cases does it enable?
- **Design** — The technical design. Contracts, responsibilities, interfaces.
- **Alternatives** — What else was considered and why it was rejected.
- **Unresolved Questions** — Open questions for future work.

### Lifecycle

1. Open an issue on GitHub describing the feature or design problem.
2. Implement it. Iterate through PRs until it's merged.
3. Once merged, write the RFC documenting the decision and add it to `SUMMARY.md`.

The RFC number is the issue number or the PR number that introduced the feature.
RFCs are written *after* implementation, not before — they record decisions that
were made, not proposals for decisions to come.
