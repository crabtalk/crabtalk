//! Session I/O benchmarks: append throughput and load latency.

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::sync::Arc;
use wcore::{
    model::HistoryEntry,
    repos::{SessionHandle, SessionRepo, mem::InMemorySessionRepo},
};

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

/// Create a fresh `InMemorySessionRepo` with `n` messages already
/// persisted, and return the repo + handle for replay.
fn prepopulate_session(n: usize) -> (Arc<InMemorySessionRepo>, SessionHandle) {
    let repo = Arc::new(InMemorySessionRepo::new());
    let handle = repo.create("bench", "bench").unwrap();
    repo.append_messages(&handle, &generate_messages(n))
        .unwrap();
    (repo, handle)
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
                        let repo = Arc::new(InMemorySessionRepo::new());
                        let handle = repo.create("bench", "bench").unwrap();
                        (repo, handle)
                    },
                    |(repo, handle)| {
                        repo.append_messages(&handle, messages).unwrap();
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
        let (repo, handle) = prepopulate_session(size);
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(repo, handle),
            |b, (repo, handle)| {
                b.iter(|| repo.load(handle).unwrap());
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_append, bench_load_context);
criterion_main!(benches);
