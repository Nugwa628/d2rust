//! The in-app GPU compositor: a system of Bevy's render graph (the
//! `RenderGraph` schedule of the render world, set
//! `RenderGraphSystems::Render`, before the camera driver) that runs the
//! compute compositor ([`Gpu::encode_rgba`]) on Bevy's own device and
//! copies its RGBA rows into the presented image's texture, and the two
//! presentation layers into theirs when the job has them
//! ([`LayerTargets`], for smooth scrolling between ticks,
//! `super::smooth`). The presented images never leave the GPU; the index
//! framebuffer is read back once per job ([`NodeIndices`]) because the
//! game frame persists and is the next frame's base, which the main
//! world's `FrameCycle` holds.
//!
//! The read-back reaches the main world a frame late while the render
//! world works on the frame before the main world's (Bevy's pipelined
//! rendering). So that the main world can still build a frame on every
//! display frame, a job may instead be composed over the index output of
//! the job before it, kept on the GPU ([`ComposeJob::base_after`]): the
//! same pixels the read-back would bring. The node notes each composed job
//! in [`NodeComposed`], which the main world checks before it asks for
//! that. Only a job that needs its read-back at once
//! ([`ComposeJob::sync_readback`]) makes the render world wait for the GPU.
//!
//! The main world packs the frame ([`super::GpuAtlas::pack`]) and hands
//! the render world a [`ComposeJob`]: the packed buffers, the atlas pages
//! (shared, replaced only when a frame set was added), the palette and the
//! target images. The node owns the atlas array texture and, when the
//! version changes, writes only the pages that changed (a page the main
//! world did not touch is the same shared page). Pixels are the compositor's:
//! this module moves bytes and decides none.
//!
//! When the job carries the native UI pass ([`UiJob`],
//! `gpu_compositor::ui_pass`), the node records it right after the
//! frame's compose, over the same atlas, map table and palette, into the
//! pass's two output-sized images (`super::native_ui`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::core_pipeline::schedule::camera_driver;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{
    RenderContext, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use d2_formats::palette::Palette;

use crate::frames::atlas::AtlasPage;
use crate::gpu_compositor::device::{FrameInputs, ResidentAtlas};
use crate::gpu_compositor::ui_pass::{UiPacked, UiPass};
use crate::gpu_compositor::{Gpu, Packed};

/// One frame for the node, extracted to the render world when it changes.
#[derive(Resource, Clone)]
pub struct ComposeJob {
    /// Job number, one per built frame: a job is composed once.
    pub seq: u64,
    /// The bridge frame it shows (diagnostics).
    pub frame: u64,
    pub packed: Arc<Packed>,
    /// The atlas pages; a page that did not change since the last job is
    /// the same shared page.
    pub pages: Arc<Vec<Arc<AtlasPage>>>,
    /// Changes whenever `pages` changes (a frame set was added).
    pub pages_version: u64,
    pub palette: Arc<Palette>,
    /// Compose over the index output of job `n` (the job before this one),
    /// still on the GPU, instead of `packed`'s base: `n`'s indices have not
    /// come back to the main world yet. `None`: the packed base.
    pub base_after: Option<u64>,
    /// The main world waits for this job's indices before it builds the
    /// next frame (a check run's exact frame cycle): the render world waits
    /// for the GPU so they arrive in this render frame.
    pub sync_readback: bool,
    pub target: Handle<Image>,
    /// Where the presentation layers go; `None`: not composed into images.
    pub layers: Option<LayerTargets>,
    /// The native UI pass of this frame; `None`: the frame is presented
    /// with its own UI.
    pub ui: Option<UiJob>,
}

/// The native UI pass of a frame: its packed UI items and the two images
/// it writes (`super::native_ui::NativeUiTargets`, the output's size).
#[derive(Clone)]
pub struct UiJob {
    pub packed: Arc<UiPacked>,
    pub cover: Handle<Image>,
    pub add: Handle<Image>,
}

/// The images of the two presentation layers, each the frame's size.
#[derive(Clone, Debug)]
pub struct LayerTargets {
    /// The world layer (map, units, missiles).
    pub world: Handle<Image>,
    /// The screen layer (UI, the local player), transparent elsewhere.
    pub screen: Handle<Image>,
    /// The world layer of the tick before: each tick frame copies the
    /// world layer here before it writes its own. Between ticks the strip
    /// the sliding world layer uncovers at the trailing edges is filled
    /// from it (`super::smooth::prev_uv`).
    pub prev: Handle<Image>,
}

/// A display frame between ticks for the node: the tick frame's draw list
/// with its moving units moved ([`super::unit_motion`]), composed over
/// that tick frame's inputs into the presentation layers only. It never
/// writes the presented frame and is never read back, so the game frame
/// stays the tick's.
#[derive(Resource, Clone)]
pub struct MotionJob {
    /// Job number: a job is composed once.
    pub seq: u64,
    /// The [`ComposeJob::seq`] of the tick frame it moves; a job over
    /// another frame than the node's last is skipped.
    pub tick_seq: u64,
    /// The draw list, packed with [`Packed::with_items`].
    pub packed: Arc<Packed>,
    /// The tick job's `pages_version`.
    pub pages_version: u64,
    pub layers: LayerTargets,
}

impl ExtractResource for MotionJob {
    type Source = MotionJob;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

impl ExtractResource for ComposeJob {
    type Source = ComposeJob;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// Frames the node has composed into the target, shared by both worlds
/// (logs and tests: the main world reads it).
#[derive(Resource, Clone, Default)]
pub struct NodeRuns(pub Arc<AtomicU64>);

/// The `seq` of the last job the node composed (0: none yet), shared by
/// both worlds: a job composed over it ([`ComposeJob::base_after`]) can
/// follow.
#[derive(Resource, Clone, Default)]
pub struct NodeComposed(pub Arc<AtomicU64>);

impl NodeComposed {
    /// Whether job `seq` is the last one the node composed.
    pub fn is(&self, seq: u64) -> bool {
        self.0.load(Ordering::Acquire) == seq
    }

    /// Whether job `seq` is the last one the node composed, waiting up to
    /// `max` while the job before it is the last one (`seq` was handed
    /// over and the render world is about to compose it). Anything else
    /// answers at once.
    pub fn wait_for(&self, seq: u64, max: Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            let last = self.0.load(Ordering::Acquire);
            if last == seq {
                return true;
            }
            if last.checked_add(1) != Some(seq) || start.elapsed() >= max {
                return false;
            }
            std::thread::sleep(Duration::from_micros(100));
        }
    }
}

/// How long the main world waits for the render world to compose the job
/// before before it builds a frame over that job's GPU output, rather than
/// skipping this display frame's new tick ([`NodeComposed::wait_for`]).
pub const CHAIN_WAIT: Duration = Duration::from_millis(2);

impl NodeRuns {
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

/// The index framebuffer of the last composed job, read back: `(seq,
/// indices)`, one byte per pixel, row-major. Shared by both worlds: the
/// render world fills it once the copy is mapped, the main world takes it
/// to commit the frame to its `FrameCycle`.
#[derive(Resource, Clone, Default)]
pub struct NodeIndices(pub Arc<Mutex<Option<JobIndices>>>);

/// A job's `seq` and its read-back indices.
pub type JobIndices = (u64, Vec<u8>);

/// How long the render world waits for the GPU to finish a frame that
/// holds an index read-back, so that the indices reach the main world in
/// the same render frame instead of on a later poll. The compose of one
/// game frame takes the GPU about a millisecond; a slower GPU frame falls
/// back to the next poll.
pub const READBACK_WAIT: Duration = Duration::from_millis(20);

impl NodeIndices {
    /// Takes the read-back indices of job `seq`, if they have arrived.
    pub fn take(&self, seq: u64) -> Option<Vec<u8>> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match slot.take() {
            Some((s, indices)) if s == seq => Some(indices),
            other => {
                *slot = other;
                None
            }
        }
    }
}

/// The node's own GPU state (render world).
#[derive(Resource, Default)]
struct NodeState {
    gpu: Option<Gpu>,
    /// The atlas array and the pages version it holds.
    atlas: Option<(u64, ResidentAtlas)>,
    /// The last composed job's `seq`.
    last: Option<u64>,
    /// The copy of the last job's indices, to map after submission, and
    /// whether to wait for it ([`ComposeJob::sync_readback`]).
    readback: Option<(u64, wgpu::Buffer, bool)>,
    /// The last composed job's `seq` and its index output, the base of a
    /// job composed over it ([`ComposeJob::base_after`]).
    last_indices: Option<(u64, wgpu::Buffer)>,
    /// The last composed job's `seq` and its uploaded inputs, for the
    /// motion jobs over that frame.
    inputs: Option<(u64, FrameInputs)>,
    /// The last composed motion job's `seq`.
    motion_last: Option<u64>,
    /// The native UI pass's pipeline, made on first use.
    ui_pass: Option<UiPass>,
}

/// A game ended (`app::session`); the next one composes from
/// a fresh atlas and job sequence (the device's pipelines stay).
fn reset_on_new_game(
    mut commands: Commands,
    mut state: ResMut<NodeState>,
    composed: Res<NodeComposed>,
    mut seen: Local<u64>,
    generation: bevy::render::Extract<Option<Res<crate::app::session::GameGeneration>>>,
) {
    let now = generation.as_ref().map_or(0, |g| g.0);
    if now != *seen {
        *seen = now;
        composed.0.store(0, Ordering::Release);
        state.atlas = None;
        state.last = None;
        state.readback = None;
        state.last_indices = None;
        state.inputs = None;
        state.motion_last = None;
        commands.remove_resource::<ComposeJob>();
        commands.remove_resource::<MotionJob>();
    }
}

/// Adds the node and the job's extraction when there is a render world.
/// Returns whether it did (no render world: no node, the CPU reference is
/// presented).
pub fn add_node(app: &mut App) -> bool {
    let runs = NodeRuns::default();
    let indices = NodeIndices::default();
    let composed = NodeComposed::default();
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return false;
    };
    render_app
        .insert_resource(runs.clone())
        .insert_resource(indices.clone())
        .insert_resource(composed.clone())
        .init_resource::<NodeState>()
        .add_systems(
            RenderGraph,
            (
                compose_node
                    .in_set(RenderGraphSystems::Render)
                    .before(camera_driver),
                motion_node
                    .in_set(RenderGraphSystems::Render)
                    .after(compose_node)
                    .before(camera_driver),
            ),
        )
        .add_systems(Render, map_indices.in_set(RenderSystems::Cleanup))
        .add_systems(bevy::render::ExtractSchedule, reset_on_new_game);
    app.insert_resource(runs)
        .insert_resource(indices)
        .insert_resource(composed)
        .add_plugins(ExtractResourcePlugin::<ComposeJob>::default())
        .add_plugins(ExtractResourcePlugin::<MotionJob>::default());
    true
}

/// Bytes per presented pixel (RGBA8).
const PIXEL: u32 = 4;

#[allow(clippy::too_many_arguments)]
fn compose_node(
    job: Option<Res<ComposeJob>>,
    mut state: ResMut<NodeState>,
    runs: Res<NodeRuns>,
    composed: Res<NodeComposed>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) -> Result {
    let Some(job) = job else { return Ok(()) };
    if state.last == Some(job.seq) {
        return Ok(());
    }
    // The target is uploaded during the render world's prepare step; until
    // it is, wait (the job stays and is composed next frame).
    let Some(target) = images.get(&job.target) else {
        return Ok(());
    };
    let state = &mut *state;
    let gpu = state
        .gpu
        .get_or_insert_with(|| Gpu::from_device(device.wgpu_device().clone(), (**queue.0).clone()));
    if state.atlas.as_ref().map(|a| a.0) != Some(job.pages_version) {
        let mut resident = state.atlas.take().map(|(_, r)| r);
        gpu.sync_atlas(&mut resident, &job.pages);
        state.atlas = resident.map(|r| (job.pages_version, r));
    }
    let (_, atlas) = state.atlas.as_ref().ok_or("atlas missing")?;
    let p = job.packed.params;
    let size = target.texture_descriptor.size;
    if (size.width, size.height) != (p.width, p.height) {
        return Err(format!(
            "target is {}x{}, the frame {}x{}",
            size.width, size.height, p.width, p.height
        )
        .into());
    }
    // A layer target not uploaded yet waits like the frame's own.
    let layers = match &job.layers {
        Some(l) => match (
            images.get(&l.world),
            images.get(&l.screen),
            images.get(&l.prev),
        ) {
            (Some(w), Some(s), Some(v)) => Some((w, s, v)),
            _ => return Ok(()),
        },
        None => None,
    };
    // So do the native UI pass's images.
    let ui = match &job.ui {
        Some(u) => match (images.get(&u.cover), images.get(&u.add)) {
            (Some(c), Some(a)) => Some((u, c, a)),
            _ => return Ok(()),
        },
        None => None,
    };
    // The job before's index output, when this job is composed over it.
    // The main world asks for that only once it saw that job composed
    // ([`NodeComposed`]); should it not be the last one (or not of this
    // frame's size), the packed (older) base is used rather than a wrong
    // buffer.
    let base = job.base_after.and_then(|after| {
        let found = state
            .last_indices
            .as_ref()
            .filter(|(seq, b)| *seq == after && b.size() == job.packed.pixel_count() as u64 * 4)
            .map(|(_, b)| b.clone());
        if found.is_none() {
            warn_once!(
                "world view: a job asked for the base of job {after}, not the last composed"
            );
        }
        found
    });
    let encoder = ctx.command_encoder();
    let Some(out) = gpu.encode_rgba_over(
        encoder,
        &job.packed,
        atlas.texture(),
        atlas.pages(),
        &job.palette,
        base.as_ref(),
    )?
    else {
        return Ok(());
    };
    let indices = &out.indices;
    let staging = device.wgpu_device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("indices readback"),
        size: indices.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(indices, 0, &staging, 0, indices.size());
    copy_rows(encoder, &out.rgba, &target.texture, p.width, p.height);
    if let Some((world, screen, prev)) = layers {
        // The tick before's world layer (the last one shown) is kept for
        // the trailing edges of the next blend.
        let size = world.texture_descriptor.size;
        if prev.texture_descriptor.size == size {
            encoder.copy_texture_to_texture(
                world.texture.as_image_copy(),
                prev.texture.as_image_copy(),
                size,
            );
        }
        for (buffer, image) in [(&out.world, world), (&out.screen, screen)] {
            let size = image.texture_descriptor.size;
            if (size.width, size.height) != (p.width, p.height) {
                return Err("a layer target differs from the frame's size".into());
            }
            copy_rows(encoder, buffer, &image.texture, p.width, p.height);
        }
    }
    if let Some((ui, cover, add)) = ui {
        let want = (ui.packed.params.width, ui.packed.params.height);
        for image in [cover, add] {
            let size = image.texture_descriptor.size;
            if (size.width, size.height) != want {
                return Err("a native UI target differs from the pass's size".into());
            }
        }
        let palette = out.inputs.palette().ok_or("the frame has no palette")?;
        let pass = state
            .ui_pass
            .get_or_insert_with(|| UiPass::new(device.wgpu_device()));
        let view = |image: &GpuImage| image.texture.create_view(&Default::default());
        pass.encode(
            device.wgpu_device(),
            encoder,
            &ui.packed,
            atlas.texture(),
            out.inputs.maps(),
            palette,
            &view(cover),
            &view(add),
        )?;
    }
    state.inputs = Some((job.seq, out.inputs));
    state.last = Some(job.seq);
    state.readback = Some((job.seq, staging, job.sync_readback));
    state.last_indices = Some((job.seq, out.indices));
    composed.0.store(job.seq, Ordering::Release);
    runs.0.fetch_add(1, Ordering::AcqRel);
    Ok(())
}

/// Composes a [`MotionJob`] into the presentation layers, over the inputs
/// of the tick frame it moves. A job over another frame than the last
/// composed one (a newer tick frame got there first) is skipped; the main
/// world sends a new one.
fn motion_node(
    job: Option<Res<MotionJob>>,
    mut state: ResMut<NodeState>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) -> Result {
    let Some(job) = job else { return Ok(()) };
    if state.motion_last == Some(job.seq) {
        return Ok(());
    }
    let state = &mut *state;
    let (Some(gpu), Some((seq, inputs)), Some((version, atlas))) =
        (&state.gpu, &state.inputs, &state.atlas)
    else {
        return Ok(());
    };
    if *seq != job.tick_seq || *version != job.pages_version {
        return Ok(());
    }
    let (Some(world), Some(screen)) = (
        images.get(&job.layers.world),
        images.get(&job.layers.screen),
    ) else {
        return Ok(());
    };
    let p = job.packed.params;
    let encoder = ctx.command_encoder();
    let Some((world_out, screen_out)) =
        gpu.encode_layers(encoder, &job.packed, atlas.texture(), atlas.pages(), inputs)?
    else {
        return Ok(());
    };
    for (buffer, image) in [(&world_out, world), (&screen_out, screen)] {
        let size = image.texture_descriptor.size;
        if (size.width, size.height) != (p.width, p.height) {
            return Err("a layer target differs from the frame's size".into());
        }
        copy_rows(encoder, buffer, &image.texture, p.width, p.height);
    }
    state.motion_last = Some(job.seq);
    Ok(())
}

/// Copies an RGBA8 output buffer into `texture`, one copy per row: the
/// buffer's rows are `width × 4` bytes, which need not meet the 256-byte
/// row alignment of a multi-row copy.
fn copy_rows(
    encoder: &mut wgpu::CommandEncoder,
    buffer: &wgpu::Buffer,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) {
    let row = u64::from(width * PIXEL);
    for y in 0..height {
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: u64::from(y) * row,
                    bytes_per_row: None,
                    rows_per_image: None,
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// After the frame's commands were submitted: maps the copy of the last
/// job's indices and hands them to [`NodeIndices`] (one u32 word per
/// pixel holding 0..=255: the low byte is the index). For a job whose
/// indices the main world waits for ([`ComposeJob::sync_readback`]) the
/// device is then waited on (up to [`READBACK_WAIT`]) so they arrive in
/// this render frame; a read-back left to a later poll arrives two or three
/// display frames on. Otherwise the render world goes on at once: the main
/// world builds the next frame over the GPU's own copy meanwhile
/// ([`ComposeJob::base_after`]).
fn map_indices(mut state: ResMut<NodeState>, out: Res<NodeIndices>, device: Res<RenderDevice>) {
    let Some((seq, buffer, wait)) = state.readback.take() else {
        return;
    };
    let slot = out.0.clone();
    let mapped = buffer.clone();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |res| {
        if let Err(e) = res {
            error!("world view: index readback of job {seq} failed: {e}");
            return;
        }
        let indices: Vec<u8> = mapped
            .slice(..)
            .get_mapped_range()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| w[0])
            .collect();
        mapped.unmap();
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some((seq, indices));
    });
    if !wait {
        return;
    }
    // A timeout leaves the mapping to Bevy's own poll on a later frame.
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(READBACK_WAIT),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Indices of another job are neither taken nor dropped.
    #[test]
    fn other_jobs_indices_stay() {
        let indices = NodeIndices::default();
        *indices.0.lock().unwrap() = Some((3, vec![9]));
        assert_eq!(indices.take(4), None);
        assert_eq!(indices.take(3), Some(vec![9]));
        assert_eq!(indices.take(3), None);
    }

    /// A composed job answers at once; the job in flight is waited for
    /// up to the limit; an older one is not waited for.
    #[test]
    fn the_main_world_waits_only_for_the_job_in_flight() {
        let c = NodeComposed::default();
        c.0.store(4, Ordering::Release);
        assert!(c.wait_for(4, Duration::ZERO));
        let start = std::time::Instant::now();
        assert!(!c.wait_for(6, Duration::from_millis(50)));
        assert!(start.elapsed() < Duration::from_millis(40));
        assert!(!c.wait_for(5, Duration::from_millis(5)));
        assert!(start.elapsed() >= Duration::from_millis(5));
        let late = c.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2));
            late.0.store(5, Ordering::Release);
        });
        assert!(c.wait_for(5, Duration::from_secs(5)));
        t.join().unwrap();
    }

    /// The render world's wait stays short next to a tick (40 ms at the
    /// base rate): it only waits for a frame the GPU is finishing.
    #[test]
    fn the_readback_wait_is_shorter_than_a_tick() {
        assert!(READBACK_WAIT < Duration::from_millis(40));
    }
}
