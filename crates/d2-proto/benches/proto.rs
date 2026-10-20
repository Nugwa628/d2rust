//! Performance baselines of `d2-proto` (criterion): the C→S classifier, the
//! S→C buffer split and typed decode / encode. Not run in CI; run with
//! `cargo bench -p d2-proto`.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use d2_proto::client::{SelectSkill, Walk};
use d2_proto::server::SetStatWord;
use d2_proto::transport::{classify_client, split_server_buffer};
use d2_proto::FixedMessage;

/// One C→S message per game id whose size rule is a fixed byte count: the
/// id then zeros, as long as the transport size says.
fn client_messages() -> Vec<Vec<u8>> {
    (0u8..0x67)
        .filter_map(|id| {
            let probe = [id, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            match d2_proto::transport::client_size(&probe) {
                d2_proto::schema::Size::Bytes(n) if n > 0 && n <= probe.len() => {
                    Some(probe[..n].to_vec())
                }
                _ => None,
            }
        })
        .collect()
}

fn bench_proto(c: &mut Criterion) {
    let msgs = client_messages();
    assert!(msgs.len() > 20, "{} messages", msgs.len());
    let mut g = c.benchmark_group("proto");
    g.bench_function("classify_client_every_fixed_id", |b| {
        b.iter(|| {
            for m in &msgs {
                black_box(classify_client(black_box(m)));
            }
        })
    });

    // An S→C flush: 500 stat updates (4 bytes each) in one buffer.
    let mut buf = Vec::new();
    for i in 0..500u16 {
        buf.extend_from_slice(
            &SetStatWord {
                stat: (i % 200) as u8,
                value: i,
            }
            .encode(),
        );
    }
    g.bench_function("split_server_buffer_500_messages", |b| {
        b.iter(|| {
            black_box(
                split_server_buffer(black_box(&buf))
                    .expect("split")
                    .messages
                    .len(),
            )
        })
    });

    let walk = Walk {
        x: 0x1234,
        y: 0x2345,
    }
    .encode();
    let skill = SelectSkill {
        skill: 5,
        left: true,
        item: u32::MAX,
    }
    .encode();
    g.bench_function("decode_encode_walk_and_select_skill", |b| {
        b.iter(|| {
            let w = Walk::decode(black_box(&walk)).expect("walk");
            let s = SelectSkill::decode(black_box(&skill)).expect("skill");
            (w.encode(), s.encode())
        })
    });
    g.finish();
}

criterion_group!(benches, bench_proto);
criterion_main!(benches);
