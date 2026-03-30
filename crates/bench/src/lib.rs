//! Shared helpers for crabtalk benchmarks.

pub mod mock_mcp;
pub mod task;

// ── Corpus generation for BM25 benchmarks ───────────────────────

const VOCABULARY: &[&str] = &[
    "agent",
    "memory",
    "session",
    "tool",
    "config",
    "daemon",
    "runtime",
    "protocol",
    "transport",
    "gateway",
    "provider",
    "model",
    "stream",
    "request",
    "response",
    "message",
    "channel",
    "dispatch",
    "handler",
    "registry",
    "schema",
    "context",
    "history",
    "compact",
    "reload",
    "skill",
    "recall",
    "forget",
    "search",
    "index",
    "token",
    "prompt",
    "system",
    "event",
    "broadcast",
    "subscribe",
    "connect",
    "socket",
    "listen",
    "accept",
    "frame",
    "encode",
    "decode",
    "serialize",
    "parse",
    "route",
    "filter",
    "scope",
    "delegate",
    "shutdown",
    "startup",
    "uptime",
    "cron",
    "schedule",
    "trigger",
    "execute",
    "install",
    "package",
    "service",
    "process",
    "spawn",
    "async",
    "future",
    "result",
    "error",
    "retry",
    "timeout",
    "buffer",
    "flush",
    "write",
    "append",
    "persist",
    "storage",
    "entry",
    "query",
    "score",
    "rank",
    "limit",
    "offset",
    "batch",
    "iterate",
    "collect",
    "aggregate",
    "merge",
    "split",
    "chunk",
    "cache",
    "evict",
    "capacity",
    "threshold",
    "compress",
    "expand",
    "validate",
    "enforce",
    "restrict",
    "allow",
    "deny",
    "bridge",
    "adapter",
    "client",
    "server",
    "endpoint",
    "webhook",
    "notify",
    "platform",
    "integration",
    "extension",
    "plugin",
    "component",
];

/// Generate a deterministic corpus of `n` documents for BM25 benchmarks.
///
/// Each document is 20-40 words selected from a fixed vocabulary using
/// index-based spreading. The query "agent memory recall session" will
/// match ~10% of documents.
pub fn generate_corpus(n: usize) -> Vec<(usize, String)> {
    (0..n)
        .map(|i| {
            let word_count = 20 + (i * 7) % 21; // 20-40 words
            let words: Vec<&str> = (0..word_count)
                .map(|j| {
                    let idx = (i * 31 + j * 17) % VOCABULARY.len();
                    VOCABULARY[idx]
                })
                .collect();
            (i, words.join(" "))
        })
        .collect()
}
