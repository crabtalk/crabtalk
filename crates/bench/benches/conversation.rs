//! Session I/O benchmarks: append throughput and load latency.

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use wcore::{model::HistoryEntry, repos::Storage, test_utils::InMemoryStorage};

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

/// Create a fresh `InMemoryStorage` with `n` messages already
/// persisted, and return the storage + handle for replay.
fn prepopulate_session(n: usize) -> (InMemoryStorage, wcore::repos::SessionHandle) {
    let storage = InMemoryStorage::new();
    let handle = storage.create_session("bench", "bench").unwrap();
    storage
        .append_session_messages(&handle, &generate_messages(n))
        .unwrap();
    (storage, handle)
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
                        let storage = InMemoryStorage::new();
                        let handle = storage.create_session("bench", "bench").unwrap();
                        (storage, handle)
                    },
                    |(storage, handle)| {
                        storage.append_session_messages(&handle, messages).unwrap();
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
        let (storage, handle) = prepopulate_session(size);
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(storage, handle),
            |b, (storage, handle)| {
                b.iter(|| storage.load_session(handle).unwrap());
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_append, bench_load_context);
criterion_main!(benches);
