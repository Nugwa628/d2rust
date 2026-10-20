//! Property tests on the client world model and the world view under
//! random unit add / remove / update sequences: the draw list build (C7
//! composite per unit, over a valid COF and an arbitrary one, poses and
//! draw keys out of range included) never panics, is deterministic, and
//! when it succeeds is sorted by key, references only resident frames,
//! and accounts for every unit; the CPU frame is pixel-identical on two
//! runs. The Bevy mirror follows the model's units through bridge frames
//! that add and remove units.
//!
//! Every rule here is a test fixture ([`Fuzz`]), not a game rule.

use crate::prop_support;

use std::collections::VecDeque;

use bevy::prelude::{App, Entity, MinimalPlugins, With};
use d2_client::assets::path::CanonicalPath;
use d2_client::bridge::dispatch::{Dispatch, HandlerError, Message};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::mirror::MirrorIndex;
use d2_client::bridge::world::ClientWorld;
use d2_client::bridge::{Bridge, BridgePlugin, BridgeResource, ClientUnit, UnitKey, UnitView};
use d2_client::composite::{ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use d2_client::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use d2_client::scene::{BlendOp, ItemTag, Rect, ShadeChain};
use d2_client::ui::{ImageRequest, TextRequest, UiDraw};
use d2_client::world_view::{
    self, TileDraw, UiRules, UiSprite, UnitPose, Unspecified, ViewAssets, ViewError, ViewRules,
};
use d2_formats::cof::{Cof, CofLayer};
use d2_formats::palette::{Palette, Rgb};
use d2_proto::PROTOCOL_VERSION;
use proptest::prelude::*;

use prop_support::{bounded, config};

const COF: &str = "data/global/chars/tst/tst.cof";
const RANDOM_COF: &str = "data/global/chars/tst/rnd.cof";

fn path(p: &str) -> CanonicalPath {
    CanonicalPath::new(p).unwrap()
}

fn set_key(component: u8, dir: u8) -> FrameSetKey {
    FrameSetKey::new(
        format!("data/global/chars/tst/c{component}.dcc"),
        FramePart::Dir(dir),
    )
    .unwrap()
}

fn layer(component: u8) -> CofLayer {
    CofLayer {
        component,
        shadow: 0,
        selectable: 1,
        override_translucency: 0,
        new_translucency: 0,
        weapon_class: *b"hth\0",
    }
}

/// Two directions, two frames, two layers (components 0 and 1).
fn valid_cof() -> Cof {
    Cof {
        layers_count: 2,
        frames: 2,
        directions: 2,
        version: 20,
        unknown: [0; 4],
        x_min: 0,
        x_max: 0,
        y_min: 0,
        y_max: 0,
        animation_rate: 256,
        layers: vec![layer(0), layer(1)],
        events: vec![0, 0],
        event_padding: Vec::new(),
        draw_order: vec![1, 0, 0, 1, 0, 1, 1, 0],
    }
}

/// An arbitrary COF: counts, layer components and draw order bytes need
/// not agree (what a damaged file parses to before the composite checks).
fn random_cof() -> impl Strategy<Value = Cof> {
    (
        0u8..4,
        0u8..3,
        0u8..3,
        proptest::collection::vec(prop_oneof![0u8..3, any::<u8>()], 0..5),
        proptest::collection::vec(prop_oneof![0u8..3, any::<u8>()], 0..40),
    )
        .prop_map(
            |(layers_count, frames, directions, comps, draw_order)| Cof {
                layers_count,
                frames,
                directions,
                version: 20,
                unknown: [0; 4],
                x_min: 0,
                x_max: 0,
                y_min: 0,
                y_max: 0,
                animation_rate: 256,
                layers: comps.into_iter().map(layer).collect(),
                events: vec![0; usize::from(frames)],
                event_padding: Vec::new(),
                draw_order,
            },
        )
}

fn palette() -> Palette {
    let mut p = Palette {
        colors: [Rgb::default(); 256],
    };
    for (i, c) in p.colors.iter_mut().enumerate() {
        let i = i as u8;
        *c = Rgb {
            r: i,
            g: 255 - i,
            b: i ^ 0x5a,
        };
    }
    p
}

/// Frame sets of components 0..3, directions 0..3, two frames each
/// (sizes and offsets from the strategy; pixels non-zero patterns).
fn assets(cof: Cof, sizes: &[(u32, u32, i32, i32)]) -> ViewAssets {
    let mut a = ViewAssets::new(palette());
    a.cofs.insert(path(COF), valid_cof());
    a.cofs.insert(path(RANDOM_COF), cof);
    let mut n = 0;
    for c in 0..3u8 {
        for d in 0..3u8 {
            let frames = (0..2)
                .map(|_| {
                    let (w, h, xo, yo) = sizes[n % sizes.len()];
                    n += 1;
                    let px = (0..w * h)
                        .map(|i| (i as u8).wrapping_add(c * 16 + d))
                        .collect();
                    IndexFrame::new(w, h, xo, yo, px).unwrap()
                })
                .collect();
            a.frames.insert(set_key(c, d), FrameSet { frames }).unwrap();
        }
    }
    a
}

/// Fixture rules driven by the unit's GUID bits: hidden units, the
/// random COF, out-of-range directions, frames, passes; placement with
/// wrapping arithmetic over the whole i32 range.
struct Fuzz;

impl ViewRules for Fuzz {
    fn tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        Ok(Vec::new())
    }

    fn unit_pose(&self, _: &ClientWorld, u: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        let g = u.key.guid;
        if g.is_multiple_of(5) {
            return Ok(None);
        }
        Ok(Some(UnitPose {
            sub_frame: 0,
            cof: path(if g & 0x8 != 0 { RANDOM_COF } else { COF }),
            dir: ((g >> 4) % 3) as usize,
            frame: ((g >> 6) % 3) as usize,
            dir64: 0,
        }))
    }

    fn unit_params(
        &self,
        _: &ClientWorld,
        u: &ClientUnit,
        _: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        Ok(UnitParams {
            pass: (u.key.guid >> 8) % 17,
            major: u.key.guid >> 12,
            minor: u32::from(u.key.unit_type),
            clip: Rect::FRAME,
            tag: ItemTag::Unit(u.key.guid),
        })
    }

    fn component_frame(
        &self,
        _: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Ok(ComponentFrame {
            set: set_key(req.slot.component, pose.dir as u8),
            index: req.frame,
        })
    }

    fn place(
        &self,
        u: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
        image: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        let g = u.key.guid as i32;
        Ok((
            g.wrapping_mul(7).wrapping_add(image.x_off) % 900,
            (g >> 3).wrapping_add(image.y_off) % 700,
        ))
    }

    fn shade(
        &self,
        _: &ClientUnit,
        _: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Ok(ShadeChain::EMPTY)
    }

    fn blend(&self, _: &ClientUnit, _: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Ok(BlendOp::Opaque)
    }
}

impl UiRules for Fuzz {
    fn ui_image(&self, req: &ImageRequest, _: &ViewAssets) -> Result<UiSprite, ViewError> {
        Ok(UiSprite {
            frame: ComponentFrame {
                set: set_key((req.image.file % 4) as u8, 0),
                index: req.image.frame as usize,
            },
            x: req.at.x,
            y: req.at.y,
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            clip: None,
        })
    }

    fn ui_text(&self, _: &TextRequest, _: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        Ok(Vec::new())
    }

    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(15)
    }
}

#[derive(Debug, Clone)]
enum Op {
    Add(UnitKey),
    /// Remove the n-th unit (mod the count).
    Remove(usize),
    /// Re-insert the n-th unit (the model's update: a unit holds only
    /// its key today).
    Update(usize),
}

fn key() -> impl Strategy<Value = UnitKey> {
    (0u8..6, prop_oneof![0u32..64, any::<u32>()])
        .prop_map(|(unit_type, guid)| UnitKey { unit_type, guid })
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => key().prop_map(Op::Add),
        1 => any::<usize>().prop_map(Op::Remove),
        1 => any::<usize>().prop_map(Op::Update),
    ]
}

fn apply(w: &mut ClientWorld, op: &Op) {
    match *op {
        Op::Add(key) => {
            w.units.insert(key, ClientUnit::new(key));
        }
        Op::Remove(n) if !w.units.is_empty() => {
            let k = *w.units.keys().nth(n % w.units.len()).unwrap();
            w.units.remove(&k);
        }
        Op::Update(n) if !w.units.is_empty() => {
            let k = *w.units.keys().nth(n % w.units.len()).unwrap();
            w.units.insert(k, ClientUnit::new(k));
        }
        _ => {}
    }
}

fn ui_draws(n: u8) -> Vec<UiDraw> {
    (0..n)
        .map(|i| {
            UiDraw::Image(ImageRequest {
                image: d2_client::ui::ImageRef {
                    file: u32::from(i),
                    frame: u32::from(i % 3),
                },
                at: d2_client::ui::Point::new(i32::from(i) * 40, 500),
                clip: d2_client::ui::Rect::new(0, 0, 800, 600),
                look: d2_client::ui::CelLook::PLAIN,
                call: d2_client::ui::draw::CelCall::Draw,
            })
        })
        .collect()
}

fn check_frame(w: &ClientWorld, ui: &[UiDraw], a: &ViewAssets) {
    let f1 = world_view::build(w, ui, &Fuzz, a);
    let f2 = world_view::build(w, ui, &Fuzz, a);
    match (&f1, &f2) {
        (Ok(x), Ok(y)) => assert_eq!(x, y),
        (Err(x), Err(y)) => assert_eq!(format!("{x:?}"), format!("{y:?}")),
        _ => panic!("two builds disagree: {f1:?} / {f2:?}"),
    }
    // Neutral rules: nothing drawn, every unit hidden.
    let n = world_view::build(w, &[], &Unspecified, a).expect("neutral rules never fail");
    assert!(n.items.is_empty());
    assert_eq!((n.units_drawn, n.units_hidden), (0, w.units.len()));

    let Ok(f) = f1 else { return };
    assert_eq!(f.units_drawn + f.units_hidden, w.units.len());
    assert!(
        f.items.windows(2).all(|p| p[0].key <= p[1].key),
        "not sorted by key"
    );
    for i in &f.items {
        // Every item frame is a resident frame of the store.
        a.frames.frame(i.frame).expect("item frame resident");
        a.frames
            .owner(i.frame)
            .expect("item frame has an owner set");
    }
    let img1 = world_view::compose_cpu(&f, a).unwrap();
    let img2 = world_view::compose_cpu(&f, a).unwrap();
    assert_eq!(img1.len(), 800 * 600 * 4);
    assert!(img1 == img2, "CPU frame differs between two runs");
}

proptest! {
    #![proptest_config(config(64))]

    /// Random add/remove/update sequences; the frame is built (and
    /// composed when it builds) after every step.
    #[test]
    fn build_under_random_updates(
        cof in random_cof(),
        sizes in proptest::collection::vec((0u32..9, 0u32..9, -20i32..20, -20i32..20), 1..6),
        ops in proptest::collection::vec(op(), 0..16),
        ui in 0u8..4,
    ) {
        bounded(move || {
            let a = assets(cof, &sizes);
            let ui = ui_draws(ui);
            let mut w = ClientWorld::default();
            for op in &ops {
                apply(&mut w, op);
                check_frame(&w, &ui, &a);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Bevy mirror through bridge frames

/// Test handler for S→C 0x0E–0x10 shaped messages: `[id, type, guid
/// u32]`; id 0x0E adds the unit, any other removes it.
fn units(world: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let Some(key) = msg.unit else {
        return Err(HandlerError::Invalid("no unit"));
    };
    if msg.id == 0x0E {
        world.units.insert(key, ClientUnit::new(key));
    } else {
        world.units.remove(&key);
    }
    Ok(())
}

struct Script {
    frames: VecDeque<Vec<Vec<u8>>>,
    pending: Vec<Vec<u8>>,
}

impl ServerLink for Script {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.pending = self.frames.pop_front().unwrap_or_default();
        Ok(Pumped { ticked: true })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending)
    }
}

/// The unit message of `(add, type, guid)` if 0x0E/0x0F have a unit
/// handler and a size of at least 6 in the size table; else `None`.
fn unit_message(add: bool, unit_type: u8, guid: u32) -> Option<Vec<u8>> {
    let id = if add { 0x0E } else { 0x0F };
    let m = d2_proto::transport::server_message(id)?;
    m.client_unit_handler?;
    let mut b = vec![id, unit_type];
    b.extend_from_slice(&guid.to_le_bytes());
    b.resize(0x40, 0);
    match d2_proto::transport::server_size(&b) {
        d2_proto::schema::Size::Bytes(n) if (6..=b.len()).contains(&n) => {
            b.truncate(n);
            Some(b)
        }
        _ => None,
    }
}

proptest! {
    #![proptest_config(config(24))]

    /// The mirror holds exactly one `UnitView` entity per model unit
    /// after every frame, whatever the add/remove order.
    #[test]
    fn mirror_follows_model(
        frames in proptest::collection::vec(
            proptest::collection::vec((any::<bool>(), 0u8..3, 0u32..6), 0..6),
            0..8,
        ),
    ) {
        // 0x0E and 0x0F have unit handlers and unit-sized messages in the
        // size table (`server-messages.tsv`).
        prop_assert!(unit_message(true, 0, 0).is_some() && unit_message(false, 0, 0).is_some());
        bounded(move || {
            let chunks: VecDeque<Vec<Vec<u8>>> = frames
                .iter()
                .map(|f| {
                    f.iter()
                        .map(|&(add, t, g)| unit_message(add, t, g).unwrap())
                        .collect()
                })
                .collect();
            let n = chunks.len();
            let mut d = Dispatch::empty();
            d.set(0x0E, "specs/client/bridge.md", units);
            d.set(0x0F, "specs/client/bridge.md", units);
            let link: Box<dyn ServerLink + Send + Sync> = Box::new(Script {
                frames: chunks,
                pending: Vec::new(),
            });
            let bridge = Bridge::with_dispatch(link, d).unwrap();
            let mut app = App::new();
            // A fixed host clock: no wall time.
            app.insert_resource(d2_client::bridge::mirror::ScriptedClock(std::sync::Arc::new(
                std::sync::atomic::AtomicU32::new(1000),
            )));
            app.add_plugins(MinimalPlugins)
                .add_plugins(BridgePlugin)
                .insert_resource(BridgeResource(bridge));
            for _ in 0..=n {
                app.update();
                let world = app.world_mut();
                let model: Vec<UnitKey> = world
                    .resource::<BridgeResource>()
                    .0
                    .world()
                    .units
                    .keys()
                    .copied()
                    .collect();
                let mut views: Vec<UnitKey> = world
                    .query::<&UnitView>()
                    .iter(world)
                    .map(|v| v.key)
                    .collect();
                views.sort();
                assert_eq!(views, model);
                let index: Vec<UnitKey> = world.resource::<MirrorIndex>().0.keys().copied().collect();
                assert_eq!(index, model);
                let entities = world.query_filtered::<Entity, With<UnitView>>().iter(world).count();
                assert_eq!(entities, model.len());
            }
        });
    }
}
