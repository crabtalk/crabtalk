//! BM25 recall benchmark at various corpus sizes.

use crabtalk_bench::generate_corpus;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use memory::{EntryKind, Memory, Op};

fn build_memory(size: usize) -> Memory {
    let mut mem = Memory::new();
    for (i, text) in generate_corpus(size) {
        mem.apply(Op::Add {
            name: format!("doc-{i}"),
            content: text,
            aliases: Vec::new(),
            kind: EntryKind::Note,
        })
        .unwrap();
    }
    mem
}

fn bench_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_recall");
    for size in [10, 100, 1_000, 10_000] {
        let mem = build_memory(size);
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &mem,
            |b, mem: &Memory| {
                b.iter(|| mem.search("agent memory recall session", 5));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_bm25);
criterion_main!(benches);
