//! Conversation I/O benchmarks: append throughput and load_context latency.

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::sync::Arc;
use wcore::{Conversation, MemStorage, model::HistoryEntry};

fn generate_messages(n: usize) -> Vec<HistoryEntry> {
    (0..n)
        .map(|i| {
            if i % 2 == 0 {
                HistoryEntry::user(format!("message {i}"))
            } else {
                HistoryEntry::assistant(format!("response {i}"), None, None)
            }
        })
        .collect()
}

/// Build a fresh `MemStorage`-backed conversation with `n` messages
/// already persisted, and return the storage + slug for replay.
fn prepopulate_conversation(n: usize) -> (Arc<MemStorage>, String) {
    let storage = Arc::new(MemStorage::new());
    let mut conversation = Conversation::new(1, "bench", "bench");
    conversation.ensure_slug(storage.as_ref());
    conversation.append_messages(storage.as_ref(), &generate_messages(n));
    let slug = conversation.slug.expect("slug minted by ensure_slug");
    (storage, slug)
}

fn bench_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("conversation_append");
    for size in [10, 100, 1_000, 5_000] {
        let messages = generate_messages(size);
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &messages,
            |b, messages| {
                b.iter_batched(
                    || {
                        let storage = Arc::new(MemStorage::new());
                        let mut conversation = Conversation::new(1, "bench", "bench");
                        conversation.ensure_slug(storage.as_ref());
                        (storage, conversation)
                    },
                    |(storage, mut conversation)| {
                        conversation.append_messages(storage.as_ref(), messages);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_load_context(c: &mut Criterion) {
    let mut group = c.benchmark_group("conversation_load");
    for size in [10, 100, 1_000, 5_000] {
        let (storage, slug) = prepopulate_conversation(size);
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(storage, slug),
            |b, (storage, slug)| {
                b.iter(|| Conversation::load_context(storage.as_ref(), slug).unwrap());
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_append, bench_load_context);
criterion_main!(benches);
