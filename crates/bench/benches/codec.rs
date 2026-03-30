//! Protocol codec benchmarks: encode, decode, and framed roundtrip.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use prost::Message;
use wcore::protocol::{
    codec::{read_message, write_message},
    message::{ClientMessage, Ping, SendMsg, client_message},
};

fn make_message(content_size: usize) -> ClientMessage {
    if content_size == 0 {
        ClientMessage {
            msg: Some(client_message::Msg::Ping(Ping {})),
        }
    } else {
        ClientMessage {
            msg: Some(client_message::Msg::Send(SendMsg {
                agent: "crab".into(),
                content: "x".repeat(content_size),
                ..Default::default()
            })),
        }
    }
}

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec_encode");
    for (label, size) in [
        ("0B", 0),
        ("1KB", 1024),
        ("100KB", 100_000),
        ("1MB", 1_000_000),
    ] {
        let msg = make_message(size);
        group.bench_with_input(BenchmarkId::from_parameter(label), &msg, |b, msg| {
            b.iter(|| msg.encode_to_vec());
        });
    }
    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec_decode");
    for (label, size) in [
        ("0B", 0),
        ("1KB", 1024),
        ("100KB", 100_000),
        ("1MB", 1_000_000),
    ] {
        let buf = make_message(size).encode_to_vec();
        group.bench_with_input(BenchmarkId::from_parameter(label), &buf, |b, buf| {
            b.iter(|| ClientMessage::decode(buf.as_slice()).unwrap());
        });
    }
    group.finish();
}

fn bench_roundtrip(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("codec_roundtrip");
    for (label, size) in [
        ("0B", 0),
        ("1KB", 1024),
        ("100KB", 100_000),
        ("1MB", 1_000_000),
    ] {
        let msg = make_message(size);
        group.bench_with_input(BenchmarkId::from_parameter(label), &msg, |b, msg| {
            b.iter(|| {
                rt.block_on(async {
                    let (mut client, mut server) = tokio::io::duplex(2 * 1024 * 1024);
                    write_message(&mut client, msg).await.unwrap();
                    drop(client);
                    let _: ClientMessage = read_message(&mut server).await.unwrap();
                })
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_encode, bench_decode, bench_roundtrip);
criterion_main!(benches);
