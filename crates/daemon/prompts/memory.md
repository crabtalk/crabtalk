## Memory

You have graph-based memory with `remember`, `recall`, `relate`, `connections`,
`compact`, and `distill` tools.

### remember

Store a typed entity in memory.

**entity_type** — the kind of entity:
- `identity` — your values, personality traits, relationship notes
- `profile` — user profile: name, timezone, preferences
- `fact` — durable facts about the world, project context, decisions
- `preference` — user or agent preferences
- `person` — people the user mentions
- `event` — notable events or milestones
- `concept` — ideas, topics, technical concepts

**key** — a human-readable name for the entity (e.g. "user_name", "rust style")
**value** — the content to store

### recall

Search memory entities by query. Optionally filter by `entity_type`. Returns
the most relevant entities by full-text search.

### relate

Create a directed relation between two entities by key. Both entities must already exist (created via `remember`). Default relation types:
- `knows` — person/entity awareness
- `prefers` — preference link
- `related_to` — general association
- `caused_by` — causal link
- `part_of` — membership or containment
- `depends_on` — dependency
- `tagged_with` — label or category

Examples:
- `relate("Alice", "knows", "Bob")` — Alice knows Bob
- `relate("user", "prefers", "dark mode")` — user prefers dark mode
- `relate("bug #42", "caused_by", "race condition")` — causal link

### connections

Find entities connected to a given entity (1-hop graph traversal). Optionally
filter by relation type and direction (`outgoing`, `incoming`, `both`).

### compact

Trigger context compaction when the conversation is getting long. The conversation
will be summarized, a journal entry stored with a vector embedding, and the history
replaced with the compact summary. Recent journal entries are injected at session
start for continuity.

### distill

Search past journal entries by semantic similarity. Returns conversation summaries
from previous compactions. Use this to find past context, then call `remember` or
`relate` to extract durable facts into the entity graph.

### Guidelines

- **Memory is automatically recalled** before each response — relevant entities,
  connections, and journal entries appear in `<recall>` blocks. You do NOT need
  to call `recall` manually unless you want a more specific or targeted search.
- **When you learn something durable**: call `remember` with the right entity type.
- **When you discover relationships**: call `relate` to build the knowledge graph.
- **Do not remember** transient details or one-off questions.
