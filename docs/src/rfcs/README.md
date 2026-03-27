# RFCs

RFCs are design decisions that shaped crabtalk. Each one captures a specific
problem, the decision we made, and why it matters for future work.

## When to write an RFC

When a design discussion reaches a conclusion that will govern future
development. Not every PR needs an RFC — only decisions that establish rules,
contracts, or interfaces that others need to know about before building.

## How to add one

1. Conclude the discussion (usually in GitHub Discussions).
2. Assign the next number: `NNNN-short-name.md`.
3. Write it up using the template below.
4. Add it to `SUMMARY.md`.

## Template

```markdown
- Feature Name: ...
- Start Date: YYYY-MM-DD
- Discussion: #N

## Summary

One paragraph.

## Motivation

What problem does this solve? What use cases does it enable?

## Design

The technical design. Contracts, responsibilities, interfaces.

## Alternatives

What else was considered and why it was rejected.

## Unresolved Questions

Open questions for future work.
```
