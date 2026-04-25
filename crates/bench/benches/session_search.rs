//! Session search benchmark — verifies the RFC 0185 performance budget.
//!
//! Targets at 100k indexed messages:
//! - `search` p99 ≤ 50ms
//! - `insert_message` ≤ 1ms (CPU only — storage I/O not on this path)
//! - Cold-start `rebuild` ≤ 500ms end-to-end (includes `list_sessions`
//!   + `load_session` reads against `FsStorage` on a tmpdir)
//!
//! `criterion` reports mean times by default; we run a few sizes so a
//! regression sticks out as a step change.

use crabtalk::storage::FsStorage;
use crabtalk_bench::generate_corpus;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use runtime::{
    Config, Runtime,
    sessions::{SearchOptions, SessionIndex},
};
use std::sync::Arc;
use tempfile::TempDir;
use wcore::{
    AgentConfig,
    model::{HistoryEntry, Model},
    storage::{SessionHandle, Storage},
    testing::provider::TestProvider,
};

const SESSION_SIZE: usize = 50; // messages per synthetic session

struct BenchCfg;

impl Config for BenchCfg {
    type Storage = FsStorage;
    type Provider = TestProvider;
    type Env = ();
}

fn build_index(messages: usize) -> SessionIndex {
    let mut idx = SessionIndex::new();
    let corpus = generate_corpus(messages);
    let session_count = (messages / SESSION_SIZE).max(1);
    let now = "2026-04-25T00:00:00Z";

    let mut session_ids = Vec::with_capacity(session_count);
    for s in 0..session_count {
        let handle = SessionHandle::new(format!("crab_tester_{s}"));
        let id = idx.ensure_session(&handle, "crab", "tester", "", None, now, now);
        session_ids.push(id);
    }
    for (i, (_doc_idx, text)) in corpus.into_iter().enumerate() {
        let session_id = session_ids[i % session_count];
        idx.insert_message(session_id, &HistoryEntry::user(text));
    }
    idx
}

/// Populate a real `FsStorage` rooted in a tempdir with `messages`
/// synthetic messages spread across `messages / SESSION_SIZE`
/// sessions. Returns the dir guard plus a Runtime that points at it,
/// so a benchmark iteration can call `rebuild_session_index`
/// end-to-end including I/O.
fn build_runtime_with_storage(messages: usize) -> (TempDir, Runtime<BenchCfg>) {
    let dir = TempDir::new().expect("tempdir");
    let storage = Arc::new(FsStorage::new(
        dir.path().to_path_buf(),
        dir.path().join("sessions"),
        Vec::new(),
    ));
    let session_count = (messages / SESSION_SIZE).max(1);
    let corpus = generate_corpus(messages);

    let mut handles = Vec::with_capacity(session_count);
    for s in 0..session_count {
        let h = storage
            .create_session("crab", &format!("tester_{s}"))
            .expect("create_session");
        handles.push(h);
    }
    for (i, (_, text)) in corpus.into_iter().enumerate() {
        let session_idx = i % session_count;
        let entry = HistoryEntry::user(text);
        storage
            .append_session_messages(&handles[session_idx], &[entry])
            .expect("append_session_messages");
    }

    let memory = Arc::new(parking_lot::RwLock::new(memory::Memory::new()));
    let runtime = Runtime::<BenchCfg>::new(
        Model::new(TestProvider::with_chunks(vec![])),
        Arc::new(()),
        storage,
        memory,
        wcore::ToolRegistry::new(),
    );
    runtime.add_agent(AgentConfig::new("crab"));
    (dir, runtime)
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_search/search");
    for messages in [1_000usize, 10_000, 100_000] {
        let idx = build_index(messages);
        group.bench_with_input(
            BenchmarkId::from_parameter(messages),
            &idx,
            |b, idx: &SessionIndex| {
                b.iter(|| idx.search("agent memory recall session", &SearchOptions::default()));
            },
        );
    }
    group.finish();
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_search/insert_message");
    for prepop in [0usize, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(prepop),
            &prepop,
            |b, &prepop| {
                let mut idx = build_index(prepop);
                let handle = SessionHandle::new("crab_tester_live");
                let now = "2026-04-25T00:00:00Z";
                let session_id = idx.ensure_session(&handle, "crab", "tester", "", None, now, now);
                let entry = HistoryEntry::user("a fresh message arrives with words to index");
                b.iter(|| {
                    idx.insert_message(session_id, &entry);
                });
            },
        );
    }
    group.finish();
}

fn bench_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_search/rebuild");
    // End-to-end cold-start: list_sessions + load_session per session
    // against a real `FsStorage` rooted in a tempdir, then rebuild
    // the BM25 index. Storage setup is done once outside the timer.
    for messages in [10_000usize, 100_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(messages),
            &messages,
            |b, &messages| {
                let (_dir, runtime) = build_runtime_with_storage(messages);
                b.iter(|| {
                    runtime
                        .rebuild_session_index()
                        .expect("rebuild_session_index");
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_search, bench_insert, bench_rebuild);
criterion_main!(benches);
