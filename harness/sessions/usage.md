Search past conversations when the user refers to earlier work ("what did we
decide", "we talked about this last week"), or when related context probably
exists and re-deriving it would waste the turn.

Queries are keywords, not questions — two to six of the user's own terms.

Tool output and tool-call arguments are not indexed, so anything you saw only
in a tool result cannot be found here. Tool *names* are indexed, so "sessions
where I ran bash" works.
