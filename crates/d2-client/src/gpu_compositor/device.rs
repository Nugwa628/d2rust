//! The wgpu side of the game-frame compositor: pipelines, upload,
//! dispatch, readback. Nothing here decides a pixel; it moves the
//! [`Packed`] bytes and the atlas pages to the GPU unchanged and reads the
//! framebuffer back.

use std::sync::{mpsc, Arc};

use d2_formats::palette::Palette;
use wgpu::util::DeviceExt;

use super::pack::{Packed, BASE_WIDE_WORD, ITEM_SIZE};
use super::{GpuError, SHADER, WORKGROUP};
use crate::frames::atlas::AtlasPage;
use crate::frames::PAGE_SIZE;

/// A device and queue with the compositor's three pipelines.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    compose: wgpu::ComputePipeline,
    to_rgba: wgpu::ComputePipeline,
    to_layers: wgpu::ComputePipeline,
}

/// The output buffers of one encoded frame ([`Gpu::encode_rgba`]), each
/// one u32 word per view pixel, usable as copy sources.
pub struct Encoded {
    /// The index framebuffer (0..=255 per word): the next frame's base.
    pub indices: wgpu::Buffer,
    /// The frame through the palette (RGBA8 bytes).
    pub rgba: wgpu::Buffer,
    /// The world layer through the palette.
    pub world: wgpu::Buffer,
    /// The screen layer: the frame's colour where the screen layer still
    /// contributes, else transparent.
    pub screen: wgpu::Buffer,
    /// The frame's uploaded inputs, for more draw lists over the same
    /// frame ([`Gpu::encode_layers`]).
    pub inputs: FrameInputs,
}

/// The inputs one game frame's draw lists share: the map table, the
/// base framebuffer (the previous frame and the clears) and the palette.
pub struct FrameInputs {
    maps: wgpu::Buffer,
    base: wgpu::Buffer,
    /// `base` is the previous frame's index output (one pixel per word),
    /// not the packed base ([`super::pack::BASE_WIDE_WORD`]).
    base_wide: bool,
    palette: Option<wgpu::Buffer>,
}

impl FrameInputs {
    /// The frame's map table (shade maps and blend tables).
    pub fn maps(&self) -> &wgpu::Buffer {
        &self.maps
    }

    /// The frame's palette, 256 words of r, g, b, 255, when one was given.
    pub fn palette(&self) -> Option<&wgpu::Buffer> {
        self.palette.as_ref()
    }
}

/// The frame's RGBA image and its world and screen layers, RGBA8 bytes each.
pub type LayerImages = (Vec<u8>, Vec<u8>, Vec<u8>);

/// The colour outputs of a dispatch with a palette.
struct Colours {
    rgba: wgpu::Buffer,
    world: wgpu::Buffer,
    screen: wgpu::Buffer,
}

impl Gpu {
    /// A headless device on the default adapter (no window, no surface):
    /// what verify uses. Also returns the adapter's description.
    pub fn headless() -> Result<(Self, wgpu::AdapterInfo), GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = bevy::tasks::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .map_err(|e| GpuError::Adapter(e.to_string()))?;
        let info = adapter.get_info();
        let (device, queue) =
            bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("d2 compositor"),
                ..Default::default()
            }))
            .map_err(|e| GpuError::Device(e.to_string()))?;
        Ok((Self::from_device(device, queue), info))
    }

    /// On an existing device, e.g. Bevy's (`RenderDevice::wgpu_device()`
    /// and the `RenderQueue`), for the in-app render node.
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("d2 compositor"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let compose = pipeline("compose");
        let to_rgba = pipeline("to_rgba");
        let to_layers = pipeline("to_layers");
        Gpu {
            device,
            queue,
            compose,
            to_rgba,
            to_layers,
        }
    }

    /// Composes `packed` over the atlas `pages`: the index framebuffer,
    /// `view.width × view.height` bytes, equal to `scene::compose`.
    pub fn compose(&self, packed: &Packed, pages: &[AtlasPage]) -> Result<Vec<u8>, GpuError> {
        Ok(self.run(packed, pages, None)?.0)
    }

    /// [`Gpu::compose`] plus the RGBA8 image through `palette` (equal to
    /// `scene::compose_rgba`). Returns `(indices, rgba)`.
    pub fn compose_rgba(
        &self,
        packed: &Packed,
        pages: &[AtlasPage],
        palette: &Palette,
    ) -> Result<(Vec<u8>, Vec<u8>), GpuError> {
        let (indices, colours) = self.run(packed, pages, Some(palette))?;
        Ok((indices, colours.map(|c| c.0).unwrap_or_default()))
    }

    /// [`Gpu::compose_rgba`]'s RGBA image plus the two presentation layers
    /// (world, screen), each RGBA8 bytes.
    pub fn compose_layers(
        &self,
        packed: &Packed,
        pages: &[AtlasPage],
        palette: &Palette,
    ) -> Result<LayerImages, GpuError> {
        let (_, colours) = self.run(packed, pages, Some(palette))?;
        Ok(colours.unwrap_or_default())
    }

    /// The presentation layers (world, screen; RGBA8 bytes) of the draw
    /// list `list` composed over the frame `frame` (its map table, base
    /// and clears), as the in-app node composes a display frame between
    /// ticks ([`Gpu::encode_layers`]). Also returns the GPU time of the
    /// submitted work, waited for.
    pub fn layers_over(
        &self,
        frame: &Packed,
        list: &Packed,
        pages: &[AtlasPage],
        palette: &Palette,
    ) -> Result<(Vec<u8>, Vec<u8>, std::time::Duration), GpuError> {
        let atlas = self.upload_pages(pages);
        let inputs = self.frame_inputs(frame, Some(palette));
        self.queue.submit([]);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Device(e.to_string()))?;
        let start = std::time::Instant::now();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let Some((world, screen)) =
            self.encode_layers(&mut encoder, list, &atlas, pages.len() as u32, &inputs)?
        else {
            return Ok((Vec::new(), Vec::new(), start.elapsed()));
        };
        self.queue.submit([encoder.finish()]);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Device(e.to_string()))?;
        let took = start.elapsed();
        let size = list.pixel_count() as u64 * 4;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let (w, s) = (
            self.staging(&mut encoder, &world, size),
            self.staging(&mut encoder, &screen, size),
        );
        self.queue.submit([encoder.finish()]);
        Ok((self.read(&w)?, self.read(&s)?, took))
    }

    /// Records the dispatches of `packed` into `encoder` over a resident
    /// atlas (a [`ResidentAtlas`] holding `pages` pages) and returns the
    /// output buffers ([`Encoded`]; colour rows are `width × 4` bytes).
    /// Nothing is submitted or read back; `None` for an empty view.
    pub fn encode_rgba(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &Packed,
        atlas: &wgpu::Texture,
        pages: u32,
        palette: &Palette,
    ) -> Result<Option<Encoded>, GpuError> {
        self.encode_rgba_over(encoder, packed, atlas, pages, palette, None)
    }

    /// [`Gpu::encode_rgba`] composed over `base`, the index output
    /// ([`Encoded::indices`]) of the frame before, still on the GPU,
    /// instead of the packed base: the same pixels without reading them
    /// back first. `None`: the packed base. A `base` of another size than
    /// the view is refused.
    pub fn encode_rgba_over(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &Packed,
        atlas: &wgpu::Texture,
        pages: u32,
        palette: &Palette,
        base: Option<&wgpu::Buffer>,
    ) -> Result<Option<Encoded>, GpuError> {
        if pages != packed.params.pages {
            return Err(GpuError::PageCount {
                pages: pages as usize,
                packed: packed.params.pages,
            });
        }
        if packed.pixel_count() == 0 {
            return Ok(None);
        }
        self.check_limits(packed, u64::from(pages))?;
        let inputs = match base {
            Some(base) => {
                let needed = packed.pixel_count() as u64 * 4;
                if base.size() != needed {
                    return Err(GpuError::Device(format!(
                        "a GPU base of {} bytes for a frame of {needed}",
                        base.size()
                    )));
                }
                self.frame_inputs_over(packed, Some(palette), base)
            }
            None => self.frame_inputs(packed, Some(palette)),
        };
        let (indices, colours) = self.encode_with(encoder, packed, atlas, &inputs, true);
        Ok(colours.map(|c| Encoded {
            indices,
            rgba: c.rgba,
            world: c.world,
            screen: c.screen,
            inputs,
        }))
    }

    fn run(
        &self,
        packed: &Packed,
        pages: &[AtlasPage],
        palette: Option<&Palette>,
    ) -> Result<(Vec<u8>, Option<LayerImages>), GpuError> {
        if pages.len() != packed.params.pages as usize {
            return Err(GpuError::PageCount {
                pages: pages.len(),
                packed: packed.params.pages,
            });
        }
        let pixels = packed.pixel_count() as u64;
        if pixels == 0 {
            return Ok((Vec::new(), palette.map(|_| Default::default())));
        }
        self.check_limits(packed, pages.len() as u64)?;
        let out_size = pixels * 4;
        let atlas = self.upload_pages(pages);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let (indices, colours, _) = self.encode(&mut encoder, packed, &atlas, palette);
        let read_index = self.staging(&mut encoder, &indices, out_size);
        let read_colours = colours.as_ref().map(|c| {
            [&c.rgba, &c.world, &c.screen].map(|b| self.staging(&mut encoder, b, out_size))
        });
        self.queue.submit([encoder.finish()]);

        // One u32 per pixel holding 0..=255: keep the low byte.
        let index_bytes = self.read(&read_index)?;
        let out: Vec<u8> = index_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| w[0])
            .collect();
        let colours = match read_colours {
            Some([rgba, world, screen]) => {
                Some((self.read(&rgba)?, self.read(&world)?, self.read(&screen)?))
            }
            None => None,
        };
        Ok((out, colours))
    }

    /// The device the compositor runs on.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue the compositor submits to.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The atlas pages as a texture array, for a caller that composes
    /// more than one pass over them (tests of the native UI pass).
    pub fn upload_atlas(&self, pages: &[AtlasPage]) -> wgpu::Texture {
        self.upload_pages(pages)
    }

    /// Reads a buffer filled by submitted work back to the CPU.
    pub fn read_back(
        &self,
        encoder: wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        size: u64,
    ) -> Result<Vec<u8>, GpuError> {
        let mut encoder = encoder;
        let staging = self.staging(&mut encoder, source, size);
        self.queue.submit([encoder.finish()]);
        self.read(&staging)
    }

    /// Uploads what every draw list over the frame `packed` shares: its
    /// map table, its base framebuffer and the palette.
    pub fn frame_inputs(&self, packed: &Packed, palette: Option<&Palette>) -> FrameInputs {
        self.inputs_with(packed, palette, None)
    }

    /// [`Gpu::frame_inputs`] with `base`, the previous frame's index
    /// output on the GPU (one pixel per word), as the base framebuffer.
    pub fn frame_inputs_over(
        &self,
        packed: &Packed,
        palette: Option<&Palette>,
        base: &wgpu::Buffer,
    ) -> FrameInputs {
        self.inputs_with(packed, palette, Some(base))
    }

    fn inputs_with(
        &self,
        packed: &Packed,
        palette: Option<&Palette>,
        gpu_base: Option<&wgpu::Buffer>,
    ) -> FrameInputs {
        let d = &self.device;
        let storage = wgpu::BufferUsages::STORAGE;
        let init = |label, contents: &[u8]| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: storage,
            })
        };
        FrameInputs {
            maps: init("maps", &packed.maps_bytes()),
            base: match gpu_base {
                Some(base) => base.clone(),
                None => init("base", &packed.base_bytes()),
            },
            base_wide: gpu_base.is_some(),
            palette: palette.map(|palette| {
                let words: Vec<u8> = palette
                    .colors
                    .iter()
                    .flat_map(|c| [c.r, c.g, c.b, 255])
                    .collect();
                init("palette", &words)
            }),
        }
    }

    /// Records the dispatches of a draw list `packed` over a frame whose
    /// inputs were uploaded ([`Gpu::frame_inputs`] of that frame): the
    /// compose and the presentation layers, and not the frame's own RGBA
    /// image. Returns the world and screen layer buffers; `None` for an
    /// empty view. Nothing is read back: the frame's index framebuffer is
    /// not this draw list's to commit (`world_view::unit_motion`).
    pub fn encode_layers(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &Packed,
        atlas: &wgpu::Texture,
        pages: u32,
        inputs: &FrameInputs,
    ) -> Result<Option<(wgpu::Buffer, wgpu::Buffer)>, GpuError> {
        if pages != packed.params.pages {
            return Err(GpuError::PageCount {
                pages: pages as usize,
                packed: packed.params.pages,
            });
        }
        if packed.pixel_count() == 0 || inputs.palette.is_none() {
            return Ok(None);
        }
        self.check_limits(packed, u64::from(pages))?;
        let (_, colours) = self.encode_with(encoder, packed, atlas, inputs, false);
        Ok(colours.map(|c| (c.world, c.screen)))
    }

    /// Records the compose dispatch (and with a palette the RGBA and layer
    /// ones); returns the index buffer, the colour outputs and the
    /// frame's inputs. Limits are checked by the caller.
    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &Packed,
        atlas: &wgpu::Texture,
        palette: Option<&Palette>,
    ) -> (wgpu::Buffer, Option<Colours>, FrameInputs) {
        let inputs = self.frame_inputs(packed, palette);
        let (indices, colours) = self.encode_with(encoder, packed, atlas, &inputs, true);
        (indices, colours, inputs)
    }

    /// [`Gpu::encode`] over uploaded frame inputs; `rgba`: also the
    /// frame's RGBA image (else that output is an unused 4-byte buffer).
    fn encode_with(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &Packed,
        atlas: &wgpu::Texture,
        inputs: &FrameInputs,
        rgba: bool,
    ) -> (wgpu::Buffer, Option<Colours>) {
        let d = &self.device;
        let init = |label, contents: &[u8], usage| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE;
        // The base's layout is the inputs' (which buffer is bound), so the
        // draw lists over one frame all read it the same way.
        let mut params_bytes = packed.params_bytes();
        let wide = BASE_WIDE_WORD * 4..BASE_WIDE_WORD * 4 + 4;
        params_bytes[wide].copy_from_slice(&u32::from(inputs.base_wide).to_le_bytes());
        let params = init("params", &params_bytes, wgpu::BufferUsages::UNIFORM);
        let items = init("items", &packed.items_bytes(), storage);
        let ranges = init("bin ranges", &packed.bin_ranges_bytes(), storage);
        let bin_items = init("bin items", &packed.bin_items_bytes(), storage);
        let out_size = packed.pixel_count() as u64 * 4;
        let output = |label, size| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let indices = output("indices", out_size);
        let layers = output("layers", out_size);
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource }
        }
        let compose_group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compose"),
            layout: &self.compose.get_bind_group_layout(0),
            entries: &[
                entry(0, params.as_entire_binding()),
                entry(1, items.as_entire_binding()),
                entry(2, ranges.as_entire_binding()),
                entry(3, bin_items.as_entire_binding()),
                entry(4, inputs.maps.as_entire_binding()),
                entry(5, wgpu::BindingResource::TextureView(&atlas_view)),
                entry(6, indices.as_entire_binding()),
                entry(9, inputs.base.as_entire_binding()),
                entry(10, layers.as_entire_binding()),
            ],
        });
        let groups = (
            packed.params.width.div_ceil(WORKGROUP),
            packed.params.height.div_ceil(WORKGROUP),
        );
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.compose);
            pass.set_bind_group(0, &compose_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        let colours = inputs.palette.as_ref().map(|palette| {
            let rgba = if rgba {
                let rgba = output("rgba", out_size);
                let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("to_rgba"),
                    layout: &self.to_rgba.get_bind_group_layout(0),
                    entries: &[
                        entry(0, params.as_entire_binding()),
                        entry(6, indices.as_entire_binding()),
                        entry(7, palette.as_entire_binding()),
                        entry(8, rgba.as_entire_binding()),
                        entry(10, layers.as_entire_binding()),
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.to_rgba);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
                drop(pass);
                rgba
            } else {
                output("rgba (unused)", 4)
            };
            let world = output("world rgba", out_size);
            let screen = output("screen rgba", out_size);
            let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("to_layers"),
                layout: &self.to_layers.get_bind_group_layout(0),
                entries: &[
                    entry(0, params.as_entire_binding()),
                    entry(6, indices.as_entire_binding()),
                    entry(7, palette.as_entire_binding()),
                    entry(10, layers.as_entire_binding()),
                    entry(11, world.as_entire_binding()),
                    entry(12, screen.as_entire_binding()),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.to_layers);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            drop(pass);
            Colours {
                rgba,
                world,
                screen,
            }
        });
        (indices, colours)
    }

    fn check_limits(&self, packed: &Packed, pages: u64) -> Result<(), GpuError> {
        let limits = self.device.limits();
        let storage = limits.max_storage_buffer_binding_size;
        let sizes = [
            (
                "item buffer",
                (packed.items.len().max(1) * ITEM_SIZE) as u64,
            ),
            ("bin item buffer", packed.bin_items.len() as u64 * 4),
            ("map table buffer", packed.maps.len() as u64),
            ("framebuffer", packed.pixel_count() as u64 * 4),
            (
                "base buffer",
                (packed.base.len().div_ceil(4).max(1) * 4) as u64,
            ),
        ];
        for (what, needed) in sizes {
            if needed > storage {
                return Err(GpuError::Limit {
                    what,
                    needed,
                    limit: storage,
                });
            }
        }
        let layers = u64::from(limits.max_texture_array_layers);
        if pages > layers {
            return Err(GpuError::Limit {
                what: "atlas pages",
                needed: pages,
                limit: layers,
            });
        }
        let groups = u64::from(limits.max_compute_workgroups_per_dimension);
        let needed = u64::from(
            packed
                .params
                .width
                .max(packed.params.height)
                .div_ceil(WORKGROUP),
        );
        if needed > groups {
            return Err(GpuError::Limit {
                what: "workgroups per dimension",
                needed,
                limit: groups,
            });
        }
        Ok(())
    }

    /// The pages as one R8Uint texture array ([`upload_atlas_pages`]).
    fn upload_pages(&self, pages: &[AtlasPage]) -> wgpu::Texture {
        upload_atlas_pages(&self.device, &self.queue, pages)
    }

    /// Brings the resident atlas up to `pages` and returns how many pages
    /// it wrote. Only pages that changed since the last call are written
    /// (a page the main world did not touch is the same shared page); the
    /// array grows with spare layers and shrinks when the page count drops
    /// below half its capacity. Rebuilding uploads every live page; otherwise
    /// a new frame set costs only the pages it was packed into.
    pub fn sync_atlas(
        &self,
        resident: &mut Option<ResidentAtlas>,
        pages: &[Arc<AtlasPage>],
    ) -> usize {
        let needed = pages.len() as u32;
        let limit = self.device.limits().max_texture_array_layers;
        let layers = atlas_layers(needed, limit);
        if resident.as_ref().is_none_or(|r| {
            needed > r.layers || (needed.saturating_mul(2) < r.layers && layers < r.layers)
        }) {
            *resident = Some(ResidentAtlas {
                texture: atlas_array(&self.device, layers),
                layers,
                held: Vec::new(),
            });
        }
        let Some(resident) = resident.as_mut() else {
            return 0;
        };
        let changed = changed_pages(&resident.held, pages);
        for &layer in &changed {
            write_atlas_page(
                &self.queue,
                &resident.texture,
                layer,
                &pages[layer as usize],
            );
        }
        resident.held = pages.to_vec();
        changed.len()
    }

    fn staging(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        size: u64,
    ) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, size);
        buffer
    }

    fn read(&self, buffer: &wgpu::Buffer) -> Result<Vec<u8>, GpuError> {
        let (tx, rx) = mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            // The receiver waits below; a send error cannot happen.
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let bytes = buffer.get_mapped_range(..).to_vec();
        buffer.unmap();
        Ok(bytes)
    }
}

/// The pages as one R8Uint texture array (layer = page), bytes
/// unchanged. No pages: one 1×1 zero layer that nothing reads. Used by
/// the compositor and by the front end's native UI pass.
pub fn upload_atlas_pages(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pages: &[AtlasPage],
) -> wgpu::Texture {
    let texture = atlas_array(device, pages.len() as u32);
    for (layer, page) in pages.iter().enumerate() {
        write_atlas_page(queue, &texture, layer as u32, page);
    }
    texture
}

/// An empty R8Uint texture array of `layers` atlas pages; no layers is
/// one 1×1 layer that nothing reads.
fn atlas_array(device: &wgpu::Device, layers: u32) -> wgpu::Texture {
    let side = if layers == 0 { 1 } else { PAGE_SIZE };
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: layers.max(1),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Writes one page's bytes into layer `layer` of an atlas array.
fn write_atlas_page(queue: &wgpu::Queue, texture: &wgpu::Texture, layer: u32, page: &AtlasPage) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &page.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(PAGE_SIZE),
            rows_per_image: Some(PAGE_SIZE),
        },
        wgpu::Extent3d {
            width: PAGE_SIZE,
            height: PAGE_SIZE,
            depth_or_array_layers: 1,
        },
    );
}

/// The atlas texture array the render node keeps across frames, and the
/// pages it holds (shared with the main world, so a page that did not
/// change is the very same page).
pub struct ResidentAtlas {
    texture: wgpu::Texture,
    /// Layers the array has room for (at least the pages held).
    layers: u32,
    held: Vec<Arc<AtlasPage>>,
}

impl ResidentAtlas {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The pages the array holds (what the compositor is told).
    pub fn pages(&self) -> u32 {
        self.held.len() as u32
    }
}

/// Layers to make for `needed` pages: half again plus a few spare, so a
/// growing atlas makes its array again only now and then. Growth room stops
/// at 64 layers (256 MiB) and the device's `limit`. At least `needed` layers
/// are returned so an invalid request is never silently truncated.
pub fn atlas_layers(needed: u32, limit: u32) -> u32 {
    if needed == 0 {
        return 0;
    }
    needed
        .saturating_add((needed / 2).max(4))
        .min(limit.min(64))
        .max(needed)
}

/// The layers whose page in `pages` is not the page `held` has there.
pub fn changed_pages(held: &[Arc<AtlasPage>], pages: &[Arc<AtlasPage>]) -> Vec<u32> {
    (0..pages.len())
        .filter(|&i| !held.get(i).is_some_and(|h| Arc::ptr_eq(h, &pages[i])))
        .map(|i| i as u32)
        .collect()
}
