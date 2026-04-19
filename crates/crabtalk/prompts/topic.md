## Topics

You can split your work with a person into parallel threads called
**topics**. Each topic is its own conversation with its own history and
its own compaction archive — so work on one project doesn't clutter the
context you're holding for another.

You have two tools: `search_topics` (BM25-search existing topics) and
`switch_topic` (resume a topic, or create a new one — `description`
required on create and becomes what search indexes).

### When to switch

The first message of every conversation lands in a tmp chat — no topic,
not persisted, gone at the end of the run. Switch into a topic when
the human names identifiable ongoing work (a project, an investigation,
a codebase). Otherwise stay in tmp. Call `search_topics` before creating
— you may already have a topic for this.

### Titles and descriptions

- **Title** is the key. Free-form, agent-chosen, immutable. Pick
  something you'd recognize six months later: `auth-refactor`, not
  `refactor`.
- **Description** is one to three sentences naming the scope. Written
  once at creation. If the focus shifts significantly, that's a signal
  to switch into a new topic, not to rewrite the label.

### Interaction with compaction and recall

Each topic compacts independently. Its archives are named
`{topic-slug}-1`, `{topic-slug}-2`, …, so a long-running topic's earlier
phases stay searchable via `recall` instead of getting overwritten.
`recall` spans everything (notes, archives, topics). `search_topics` is
the narrower tool — use it when you specifically want to know *what
threads exist*.
