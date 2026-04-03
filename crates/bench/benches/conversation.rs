//! Conversation I/O benchmarks: append throughput and load_context latency.

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::{io::Write, path::Path};
use wcore::{Conversation, model::Message};

fn generate_messages(n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            if i % 2 == 0 {
                Message::user(format!("message {i}"))
            } else {
                Message::assistant(format!("response {i}"), None, None)
            }
        })
        .collect()
}

/// Pre-populate a JSONL file with a meta line + n messages, return the path.
fn prepopulate_conversation(dir: &Path, n: usize) -> std::path::PathBuf {
    let path = dir.join("bench_conversation.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();

    // ConversationMeta is not re-exported, write the JSON directly.
    writeln!(
        file,
        r#"{{"agent":"bench","created_by":"bench","created_at":"2026-01-01T00:00:00Z","title":"","uptime_secs":0}}"#
    )
    .unwrap();

    for msg in &generate_messages(n) {
        writeln!(file, "{}", serde_json::to_string(msg).unwrap()).unwrap();
    }

    path
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
                        let dir = tempfile::tempdir().unwrap();
                        let mut conversation = Conversation::new(1, "bench", "bench");
                        conversation.init_file(dir.path());
                        (dir, conversation)
                    },
                    |(_dir, conversation)| {
                        conversation.append_messages(messages);
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
        let dir = tempfile::tempdir().unwrap();
        let path = prepopulate_conversation(dir.path(), size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &path, |b, path| {
            b.iter(|| Conversation::load_context(path).unwrap());
        });
    }
    group.finish();
}

criterion_group!(benches, bench_append, bench_load_context);
criterion_main!(benches);
