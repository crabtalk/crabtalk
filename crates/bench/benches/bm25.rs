//! BM25 recall benchmark at various corpus sizes.

use crabtalk::hooks::memory::bm25;
use crabtalk_bench::generate_corpus;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

fn bench_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_recall");
    for size in [10, 100, 1_000, 10_000] {
        let corpus = generate_corpus(size);
        let docs: Vec<(usize, &str)> = corpus.iter().map(|(i, s)| (*i, s.as_str())).collect();
        group.bench_with_input(BenchmarkId::from_parameter(size), &docs, |b, docs| {
            b.iter(|| bm25::score(docs, "agent memory recall session", 5));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_bm25);
criterion_main!(benches);
