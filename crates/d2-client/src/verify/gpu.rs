//! The verify harness's GPU half: [`GpuCompositor`] over the compute compositor
//! ([`crate::gpu_compositor`]) on a headless adapter. Glue only: the job's frames go into a frame
//! atlas, the job is packed against the runner's own bins (a mismatch is an error, not a
//! re-bin), and the compositor's index framebuffer and RGBA8 image are read back. The CPU side
//! it is compared with lives in the parent module.
//!
//! Stable capture: a capture is judged only once it repeats. Each job is dispatched twice; two
//! different readbacks are an error, never a result.

use super::{GpuCompositor, GpuJob, GpuOutcome};
use crate::gpu_compositor::{pack, AtlasFrames, Gpu, GpuError};

/// Atlas pages a verify job may use (the same limit as `gpu_compositor::harness`).
pub const MAX_PAGES: u32 = 4;

/// The compute compositor on a headless adapter, opened on first use so a
/// run without synthetic cases never touches the GPU.
#[derive(Default)]
pub struct Wgpu {
    state: Option<State>,
}

enum State {
    Ready(Box<Gpu>, String),
    NoAdapter(String),
    Failed(String),
}

impl Wgpu {
    /// A compositor whose adapter is not opened yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the adapter (once) and describes it: `adapter: NAME (backend,
    /// type, driver …)`, `no adapter: …` or `GPU device error: …`.
    pub fn open(&mut self) -> String {
        match self.state() {
            State::Ready(_, info) => format!("adapter: {info}"),
            State::NoAdapter(e) => format!("no adapter: {e}"),
            State::Failed(e) => format!("GPU device error: {e}"),
        }
    }

    fn state(&mut self) -> &State {
        self.state.get_or_insert_with(|| match Gpu::headless() {
            Ok((gpu, i)) => State::Ready(
                Box::new(gpu),
                format!(
                    "{} ({:?}, {:?}, driver {} {})",
                    i.name, i.backend, i.device_type, i.driver, i.driver_info
                ),
            ),
            Err(GpuError::Adapter(e)) => State::NoAdapter(e),
            Err(e) => State::Failed(e.to_string()),
        })
    }
}

impl GpuCompositor for Wgpu {
    fn compose(&mut self, job: &GpuJob<'_>) -> GpuOutcome {
        let gpu = match self.state() {
            State::Ready(gpu, _) => gpu,
            State::NoAdapter(e) => return GpuOutcome::NoAdapter(e.clone()),
            State::Failed(e) => return GpuOutcome::Error(e.clone()),
        };
        match run(gpu, job) {
            Ok(outcome) => outcome,
            Err(e) => GpuOutcome::Error(e.to_string()),
        }
    }
}

fn run(gpu: &Gpu, job: &GpuJob<'_>) -> Result<GpuOutcome, GpuError> {
    let atlas = AtlasFrames::from_images(job.frames, MAX_PAGES)?;
    let packed = pack(
        job.items,
        job.bins,
        job.frames,
        &atlas.slots,
        job.maps,
        atlas.page_count(),
    )?;
    let first = gpu.compose_rgba(&packed, atlas.pages(), job.palette)?;
    let second = gpu.compose_rgba(&packed, atlas.pages(), job.palette)?;
    if first != second {
        return Ok(GpuOutcome::Error(
            "unstable capture: two dispatches of the same job read back differently".into(),
        ));
    }
    let (indices, rgba) = first;
    Ok(GpuOutcome::Image { indices, rgba })
}
