//! Headless synthetic rendering probe: no surface, window or game assets.
//!
//! Run `cargo run -p d2-client --example lowspec_probe`. The default is at
//! least 600 measured frames and ten wall-clock seconds after setup, with
//! ten warm-up frames. `D2_LOWSPEC_FRAMES` and `D2_LOWSPEC_SECONDS` override
//! those minima. JSON records CPU phases, submit/wait, uploads, texture
//! allocation sizes and hashes. Submit/wait serializes work for measurement;
//! it is neither surface-present time nor a pipelined display benchmark.
use std::sync::Arc;
use std::time::Instant;

use d2_client::frames::{atlas::AtlasPage, PAGE_SIZE};
use d2_client::gpu_compositor::{device::atlas_layers, harness, ui_pass::*, Gpu};
use d2_client::scene::Rect;
use serde_json::json;
use sha2::{Digest, Sha256};

fn stats(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    json!({"mean_ms": samples.iter().sum::<f64>() / n as f64,
        "p50_ms": samples[n / 2], "p95_ms": samples[(n * 95 / 100).min(n - 1)],
        "max_ms": samples[n - 1], "samples": n})
}

fn resident_rgba(
    gpu: &Gpu,
    device: &wgpu::Device,
    packed: &d2_client::gpu_compositor::pack::Packed,
    resident: &d2_client::gpu_compositor::device::ResidentAtlas,
    palette: &d2_formats::palette::Palette,
) -> anyhow::Result<Vec<u8>> {
    let mut encoder = device.create_command_encoder(&Default::default());
    let out = gpu
        .encode_rgba(
            &mut encoder,
            packed,
            resident.texture(),
            resident.pages(),
            palette,
        )?
        .expect("nonempty probe");
    Ok(gpu.read_back(encoder, &out.rgba, packed.pixel_count() as u64 * 4)?)
}

fn main() -> anyhow::Result<()> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = bevy::tasks::block_on(instance.request_adapter(&Default::default()))?;
    let info = adapter.get_info();
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&Default::default()))?;
    let gpu = Gpu::from_device(device.clone(), queue.clone());
    let case = harness::stress("lowspec", 11, 3000, Rect::new(0, 0, 1068, 600));
    let (atlas, packed) = harness::prepare(&case)?;
    let pages: Vec<_> = atlas.pages().iter().cloned().map(Arc::new).collect();
    let mut resident = None;
    let uploaded = gpu.sync_atlas(&mut resident, &pages);
    let resident = resident.expect("resident atlas");
    let ui_pass = UiPass::new(&device);
    let target = || {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("1080p UI"),
            size: wgpu::Extent3d {
                width: 1920,
                height: 1080,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
    };
    let (cover, add) = (target(), target());
    let (cover_view, add_view) = (
        cover.create_view(&Default::default()),
        add.create_view(&Default::default()),
    );
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let mut world = Vec::new();
    let mut ui = Vec::new();
    let mut encode = Vec::new();
    let mut submit_wait = Vec::new();
    let mut frame = Vec::new();
    let mut upload_bytes = 0;
    let mut hitches = Vec::new();
    let epoch = Instant::now();
    let runs = std::env::var("D2_LOWSPEC_FRAMES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(600)
        .max(1);
    let seconds = std::env::var("D2_LOWSPEC_SECONDS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(10.0)
        .max(0.0);
    let mut n = 0;
    while n < runs + 10 || epoch.elapsed().as_secs_f64() < seconds {
        let start = Instant::now();
        let bins = d2_client::scene::bin(&case.items, &case.frames, &case.maps, case.view)?;
        let packed = d2_client::gpu_compositor::pack(
            &case.items,
            &bins,
            &case.frames,
            &atlas.slots,
            &case.maps,
            atlas.page_count(),
        )?
        .with_frame(&case.base(), case.plan)?;
        let world_ms = start.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let draws: Vec<_> = case
            .items
            .iter()
            .step_by(30)
            .map(|item| UiDraw {
                item,
                blend: UiBlend::Opaque,
                image: CelImage::Original,
            })
            .collect();
        let ui_packed = pack_ui(
            &draws,
            &case.frames,
            &atlas.slots,
            atlas.page_count(),
            packed.params.map_rows,
            case.view,
            UiPlacement {
                frame_w: 1068,
                frame_h: 600,
                out_w: 1920,
                out_h: 1080,
            },
        )?;
        let ui_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        let out = gpu
            .encode_rgba(
                &mut encoder,
                &packed,
                resident.texture(),
                resident.pages(),
                &case.palette,
            )?
            .expect("nonempty");
        ui_pass.encode(
            &device,
            &mut encoder,
            &ui_packed,
            resident.texture(),
            out.inputs.maps(),
            out.inputs.palette().expect("palette"),
            &cover_view,
            &add_view,
        )?;
        let command = encoder.finish();
        let encode_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        queue.submit([command]);
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let wait_ms = t.elapsed().as_secs_f64() * 1000.0;
        if n >= 10 {
            world.push(world_ms);
            ui.push(ui_ms);
            encode.push(encode_ms);
            submit_wait.push(wait_ms);
            let total_ms = start.elapsed().as_secs_f64() * 1000.0;
            frame.push(total_ms);
            if total_ms >= 50.0 {
                hitches.push(
                    json!({"frame": n, "elapsed_ms": epoch.elapsed().as_secs_f64() * 1000.0,
                    "total_ms": total_ms, "world_ms": world_ms, "ui_ms": ui_ms,
                    "encode_ms": encode_ms, "wait_ms": wait_ms}),
                );
            }
        }
        if n == 0 {
            upload_bytes = packed.params_bytes().len()
                + packed.items_bytes().len()
                + packed.bin_ranges_bytes().len()
                + packed.bin_items_bytes().len()
                + packed.maps_bytes().len()
                + packed.base_bytes().len()
                + 1024
                + ui_packed.params_bytes().len()
                + ui_packed.items_bytes().len()
                + ui_packed.bin_ranges_bytes().len()
                + ui_packed.bin_items_bytes().len();
        }
        n += 1;
    }
    let (indices, rgba) = gpu.compose_rgba(&packed, atlas.pages(), &case.palette)?;
    anyhow::ensure!(indices == case.reference()?, "index framebuffer differs");
    let allocations: Vec<_> = [1, 20, 60, 64]
        .into_iter()
        .map(|needed| {
            let layers = atlas_layers(needed, device.limits().max_texture_array_layers);
            json!({"used_pages": needed, "allocated_layers": layers,
            "allocated_mib": u64::from(layers) * u64::from(PAGE_SIZE).pow(2) / (1024 * 1024)})
        })
        .collect();
    // Actual allocation and upload through the same resident path used in play.
    let blank = Arc::new(AtlasPage {
        pixels: vec![0; (PAGE_SIZE * PAGE_SIZE) as usize],
        dirty: true,
        generation: 0,
    });
    let mut full = None;
    let mut full_pages = pages.clone();
    full_pages.resize(64, blank);
    let burst = Instant::now();
    gpu.sync_atlas(&mut full, &full_pages);
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let full_atlas_upload_ms = burst.elapsed().as_secs_f64() * 1000.0;
    let size = full.as_ref().expect("full atlas").texture().size();
    let mut full_packed = packed.clone();
    full_packed.params.pages = 64;
    let full_rgba = resident_rgba(
        &gpu,
        &device,
        &full_packed,
        full.as_ref().expect("full"),
        &case.palette,
    )?;
    anyhow::ensure!(full_rgba == rgba, "full resident atlas differs");
    let steady_atlas_upload_pages = gpu.sync_atlas(&mut full, &full_pages);
    let burst = Instant::now();
    let shrink_upload_pages = gpu.sync_atlas(&mut full, &full_pages[..20]);
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let shrink_atlas_upload_ms = burst.elapsed().as_secs_f64() * 1000.0;
    let shrunk = full.as_ref().expect("shrunk atlas").texture().size();
    full_packed.params.pages = 20;
    let shrunk_rgba = resident_rgba(
        &gpu,
        &device,
        &full_packed,
        full.as_ref().expect("shrunk"),
        &case.palette,
    )?;
    anyhow::ensure!(shrunk_rgba == rgba, "shrunk resident atlas differs");
    println!(
        "{}",
        json!({
            "adapter": format!("{} {:?} {:?}", info.name, info.backend, info.device_type),
            "debug_assertions": cfg!(debug_assertions),
            "minimum_wall_seconds": seconds,
            "native_size": [1068, 600], "ui_output_size": [1920, 1080],
            "world_items": case.items.len(), "ui_items": case.items.len().div_ceil(30),
            "world_build": stats(world), "ui_build": stats(ui), "upload_and_encode": stats(encode),
            "gpu_submit_wait": stats(submit_wait), "headless_frame": stats(frame),
            "present": "not measured: no surface", "simulation": "tools/perf/perf.py --no-window",
            "upload_bytes_per_tick": upload_bytes, "initial_atlas_upload_pages": uploaded,
            "steady_atlas_upload_pages": steady_atlas_upload_pages,
            "shrink_upload_pages": shrink_upload_pages,
            "full_atlas_upload_and_wait_ms": full_atlas_upload_ms,
            "shrink_atlas_upload_and_wait_ms": shrink_atlas_upload_ms,
            "atlas_bytes_after_64_to_20": u64::from(shrunk.width) * u64::from(shrunk.height) * u64::from(shrunk.depth_or_array_layers),
            "hitches_over_50_ms": hitches,
            "native_output_bytes": 1068_u64 * 600 * 4 * 5,
            "ui_target_bytes_1080p": 1920_u64 * 1080 * 4 * 2,
            "atlas_allocations": allocations,
            "actual_full_atlas_bytes": u64::from(size.width) * u64::from(size.height) * u64::from(size.depth_or_array_layers),
            "indices_sha256": format!("{:x}", Sha256::digest(&indices)),
            "resident_shrink_rgba_sha256": format!("{:x}", Sha256::digest(&shrunk_rgba)),
            "rgba_sha256": format!("{:x}", Sha256::digest(&rgba)),
        })
    );
    Ok(())
}
