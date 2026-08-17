Summarize the conversation so far into a compact context block that can replace
the full history. Preserve:

- Agent identity (name, personality, relationship notes from the system prompt)
- User profile (name, preferences, context from the system prompt)
- Key decisions made and their rationale
- Active tasks and their current status
- Important facts, constraints, and user preferences mentioned
- Any tool results that are still relevant to ongoing work

Omit:
- Greetings, acknowledgements, and filler
- Superseded plans or abandoned approaches
- Tool calls whose results have already been incorporated

Write in dense prose, not bullet points. The summary will become the new context
for the next part of the conversation, so it must be self-contained.
