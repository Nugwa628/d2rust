// Spec: specs/client/render-pipeline.md §A3–§A5, §A8
//! Performance baselines of the CPU compositor (criterion;
//! `docs/handoff/bench-baselines.md`): draw order, the reference compose,
//! the binned compose the GPU mirrors, and the index → RGBA step, on a
//! synthetic 800 × 600 scene. Not run in CI; `cargo bench -p d2-client
//! --bench compose`.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use d2_client::scene::{
    bin, compose, compose_binned, order, to_rgba, BlendOp, DrawItem, DrawKey, FrameId, FrameImage,
    MapId, MapTable, Rect, ShadeChain,
};
use d2_formats::palette::{Palette, Rgb};

/// Sprites in the scene: a crowded screen (tiles are not counted: the
/// floor is a handful of large items here).
const SPRITES: usize = 400;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self, n: u32) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) % n
    }
}

/// A scene of floor tiles, shaded and blended sprites, in random order.
fn scene() -> (Vec<DrawItem>, Vec<FrameImage>, MapTable) {
    let mut rng = Lcg(7);
    let mut frames = Vec::new();
    for size in [16u32, 32, 48, 64, 96] {
        // A frame with a transparent border and a filled middle.
        let pixels = (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                if x < 2 || y < 2 || x + 2 >= size || y + 2 >= size {
                    0
                } else {
                    1 + ((x ^ y) % 250) as u8
                }
            })
            .collect();
        frames.push(FrameImage {
            width: size,
            height: size,
            pixels,
        });
    }
    // Index 0..4: a shade map each (a rotation of the palette); the blend
    // table is 256 rows after them.
    let mut maps = MapTable::new();
    let shades: Vec<MapId> = (1..=4u32)
        .map(|k| {
            let mut row = [0u8; 256];
            for (i, v) in row.iter_mut().enumerate() {
                *v = (i as u32 * k % 256) as u8;
            }
            maps.push(row)
        })
        .collect();
    let table: Vec<[u8; 256]> = (0..256u32)
        .map(|s| {
            let mut row = [0u8; 256];
            for (d, v) in row.iter_mut().enumerate() {
                *v = ((s + d as u32) / 2) as u8;
            }
            row
        })
        .collect();
    let blend = maps.push(table[0]);
    for row in &table[1..] {
        maps.push(*row);
    }
    let mut items = Vec::new();
    for i in 0..SPRITES {
        let mut it = DrawItem::new(
            FrameId(rng.next(frames.len() as u32)),
            rng.next(850) as i32 - 25,
            rng.next(650) as i32 - 25,
        );
        if i % 3 == 0 {
            it.shade = ShadeChain::new(&shades[..1 + i % 4]).expect("chain");
        }
        if i % 5 == 0 {
            it.blend = BlendOp::IndexTable(blend);
        }
        it.key =
            DrawKey::new((i % 3) as u32, rng.next(1 << 20), rng.next(1 << 20), 0).expect("key");
        items.push(it);
    }
    (items, frames, maps)
}

fn palette() -> Palette {
    let mut p = Palette {
        colors: [Rgb::default(); 256],
    };
    for (i, c) in p.colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: (i * 3) as u8,
            b: (255 - i) as u8,
        };
    }
    p
}

fn bench_compose(c: &mut Criterion) {
    let (mut items, frames, maps) = scene();
    order(&mut items);
    let view = Rect::FRAME;
    let pixels = u64::from(view.width) * u64::from(view.height);
    let reference = compose(&items, &frames, &maps, view).expect("compose");
    let bins = bin(&items, &frames, &maps, view).expect("bin");
    assert_eq!(
        compose_binned(&items, &bins, &frames, &maps, view).expect("binned"),
        reference
    );
    let pal = palette();
    let mut g = c.benchmark_group("compositor");
    g.throughput(Throughput::Elements(pixels));
    g.bench_function("order_400_items", |b| {
        b.iter_batched(
            || scene().0,
            |mut v| {
                order(&mut v);
                v
            },
            criterion::BatchSize::SmallInput,
        )
    });
    g.bench_function("compose_800x600_400_items", |b| {
        b.iter(|| black_box(compose(&items, &frames, &maps, view).expect("compose")))
    });
    g.bench_function("bin_800x600_400_items", |b| {
        b.iter(|| black_box(bin(&items, &frames, &maps, view).expect("bin")))
    });
    g.bench_function("compose_binned_800x600_400_items", |b| {
        b.iter(|| black_box(compose_binned(&items, &bins, &frames, &maps, view).expect("binned")))
    });
    g.bench_function("to_rgba_800x600", |b| {
        b.iter(|| black_box(to_rgba(black_box(&reference), &pal)))
    });
    g.finish();
}

criterion_group!(benches, bench_compose);
criterion_main!(benches);
