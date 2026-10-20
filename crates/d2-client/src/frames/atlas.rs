// Spec: specs/client/render-pipeline.md (§A2 atlas), specs/client/assets.md (§A5 atlas eviction)
//! Deterministic shelf packer over R8 atlas pages, CPU side.
//!
//! This is the CPU copy of the sprite atlas: [`super::IndexFrame`] pixels
//! are packed here, and [`super::upload`] sends the changed pages to the
//! GPU. The atlas grows and is evicted in whole pages, which keeps eviction
//! trivial (clear a page, start packing it again) and lets a page keep one
//! texture for its whole life.
//!
//! Pages are `PAGE_SIZE²` bytes of palette indices, all 0 (transparent)
//! where no frame is. Placement depends only on the order of inserts: no
//! randomness, no hash iteration, so the same loads always give the same
//! slots. Every slot keeps a [`GUTTER`]-wide ring of index 0 around it,
//! page edges included, so filtered or off-by-one reads at a slot's edge
//! see transparency, never a neighbour's pixels.
//!
//! Placement rule (first fit): for each page in order, for each shelf of
//! that page in creation order, the first shelf at least as tall as the
//! frame with room left on its row takes it; else a new shelf as tall as
//! the frame opens below the page's last shelf; else the next page is
//! tried; else a new page opens, up to `max_pages`. A frame set goes in
//! whole or not at all.

/// Width and height of one atlas page (render-pipeline §A2).
pub const PAGE_SIZE: u32 = 2048;
/// Pixels of index 0 between slots and around the page edge.
pub const GUTTER: u32 = 1;
/// Largest frame side that fits a page with its gutters.
pub const MAX_SIDE: u32 = PAGE_SIZE - 2 * GUTTER;

/// Where a frame's pixels are: `w × h` at `(x, y)` on page `page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AtlasSlot {
    /// Page number.
    pub page: u32,
    /// Left edge on the page, in pixels.
    pub x: u32,
    /// Top edge on the page, in pixels.
    pub y: u32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

impl AtlasSlot {
    /// The slot of an empty (0-wide or 0-high) frame: it occupies no
    /// texels and names no live page; nothing may be read through it.
    pub const EMPTY: AtlasSlot = AtlasSlot {
        page: 0,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    /// Whether the slot covers no pixels.
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

/// Why a frame set could not be packed or an atlas operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AtlasError {
    /// A frame is larger than a page can hold with its gutters.
    #[error("frame {w}x{h} exceeds the largest slot {MAX_SIDE}x{MAX_SIDE}")]
    TooLarge { w: u32, h: u32 },
    /// The frame set does not fit even with every allowed page in use.
    #[error("atlas full: {pages} pages in use, frame set does not fit")]
    Full { pages: u32 },
    /// A frame's pixel buffer length is not `width × height`.
    #[error("frame {width}x{height} has {len} pixels")]
    PixelCount { width: u32, height: u32, len: usize },
    /// The page number does not exist.
    #[error("no atlas page {0}")]
    NoPage(u32),
    /// The atlas was asked to hold zero pages.
    #[error("max_pages must be at least 1")]
    NoPages,
}

/// One row of slots on a page: every slot on it starts at `y`, and frames
/// up to `height` tall fit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shelf {
    y: u32,
    height: u32,
    /// x of the next slot on this shelf.
    next_x: u32,
}

/// The packer state of one page: its shelves, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layout {
    shelves: Vec<Shelf>,
    /// y of the next new shelf.
    next_y: u32,
}

impl Layout {
    fn new() -> Self {
        Self {
            shelves: Vec::new(),
            next_y: GUTTER,
        }
    }

    /// Places `w × h` on this page by the first-fit rule, or `None`.
    fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        for s in &mut self.shelves {
            if h <= s.height && s.next_x + w + GUTTER <= PAGE_SIZE {
                let at = (s.next_x, s.y);
                s.next_x += w + GUTTER;
                return Some(at);
            }
        }
        if self.next_y + h + GUTTER <= PAGE_SIZE {
            let y = self.next_y;
            self.shelves.push(Shelf {
                y,
                height: h,
                next_x: GUTTER + w + GUTTER,
            });
            self.next_y += h + GUTTER;
            return Some((GUTTER, y));
        }
        None
    }
}

/// One page: its bytes and upload bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasPage {
    /// `PAGE_SIZE²` palette indices, row-major, top row first.
    pub pixels: Vec<u8>,
    /// Changed since the last upload.
    pub dirty: bool,
    /// Bumped on every clear: slots handed out before it are invalid.
    pub generation: u32,
}

impl AtlasPage {
    /// A transparent page, dirty so its first upload creates its texture.
    fn new() -> Self {
        Self {
            pixels: vec![0; (PAGE_SIZE * PAGE_SIZE) as usize],
            dirty: true,
            generation: 0,
        }
    }
}

/// The CPU copy of the atlas and its packer.
#[derive(Debug, Clone)]
pub struct Atlas {
    max_pages: u32,
    /// Packer state, one per page; always as long as `pages` once an
    /// insert has finished.
    layouts: Vec<Layout>,
    pages: Vec<AtlasPage>,
}

/// Result of [`Atlas::check`]: pixels that differ from what was put in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Slot pixels unequal to the frame's pixels.
    pub frame_mismatches: u64,
    /// Gutter-ring pixels that are not index 0.
    pub gutter_mismatches: u64,
    /// First mismatch as `(page, x, y)`, in check order.
    pub first: Option<(u32, u32, u32)>,
}

impl CheckReport {
    /// Total mismatching pixels, frames and gutters together.
    pub fn mismatches(&self) -> u64 {
        self.frame_mismatches + self.gutter_mismatches
    }

    /// Whether the atlas matched everything it was checked against.
    pub fn is_ok(&self) -> bool {
        self.mismatches() == 0
    }
}

impl Atlas {
    /// An empty atlas that may grow to `max_pages` pages (the GPU memory
    /// budget for sprites, counted in whole pages).
    pub fn new(max_pages: u32) -> Result<Self, AtlasError> {
        if max_pages == 0 {
            return Err(AtlasError::NoPages);
        }
        Ok(Self {
            max_pages,
            layouts: Vec::new(),
            pages: Vec::new(),
        })
    }

    /// The most pages the atlas may grow to.
    pub fn max_pages(&self) -> u32 {
        self.max_pages
    }

    /// The pages created so far, in page order.
    pub fn pages(&self) -> &[AtlasPage] {
        &self.pages
    }

    /// Packs every frame of `frames` and copies its pixels in, all or
    /// nothing: on error the atlas is unchanged. Returns one slot per
    /// frame, in order. Empty frames get [`AtlasSlot::EMPTY`].
    pub fn insert_set(
        &mut self,
        frames: &[super::IndexFrame],
    ) -> Result<Vec<AtlasSlot>, AtlasError> {
        self.insert_frames(&frames.iter().collect::<Vec<_>>())
    }

    /// [`Atlas::insert_set`] for frames that are not one slice (the
    /// frames a draw list needs, picked from the store).
    pub fn insert_frames(
        &mut self,
        frames: &[&super::IndexFrame],
    ) -> Result<Vec<AtlasSlot>, AtlasError> {
        for f in frames {
            if f.pixels.len() as u64 != u64::from(f.width) * u64::from(f.height) {
                return Err(AtlasError::PixelCount {
                    width: f.width,
                    height: f.height,
                    len: f.pixels.len(),
                });
            }
            if !f.is_empty() && (f.width > MAX_SIDE || f.height > MAX_SIDE) {
                return Err(AtlasError::TooLarge {
                    w: f.width,
                    h: f.height,
                });
            }
        }
        let mut layouts = self.layouts.clone();
        let mut slots = Vec::with_capacity(frames.len());
        for f in frames {
            if f.is_empty() {
                slots.push(AtlasSlot::EMPTY);
                continue;
            }
            let (w, h) = (f.width, f.height);
            let placed = layouts
                .iter_mut()
                .enumerate()
                .find_map(|(p, l)| l.place(w, h).map(|(x, y)| (p, x, y)));
            let (page, x, y) = match placed {
                Some(p) => p,
                None if (layouts.len() as u32) < self.max_pages => {
                    let mut l = Layout::new();
                    let (x, y) = l.place(w, h).ok_or(AtlasError::TooLarge { w, h })?;
                    layouts.push(l);
                    (layouts.len() - 1, x, y)
                }
                None => {
                    return Err(AtlasError::Full {
                        pages: layouts.len() as u32,
                    })
                }
            };
            slots.push(AtlasSlot {
                page: page as u32,
                x,
                y,
                w,
                h,
            });
        }
        self.layouts = layouts;
        while self.pages.len() < self.layouts.len() {
            self.pages.push(AtlasPage::new());
        }
        for (f, s) in frames.iter().zip(&slots) {
            if s.is_empty() {
                continue;
            }
            let page = &mut self.pages[s.page as usize];
            page.dirty = true;
            for row in 0..s.h {
                let src = (row * s.w) as usize;
                let dst = ((s.y + row) * PAGE_SIZE + s.x) as usize;
                page.pixels[dst..dst + s.w as usize]
                    .copy_from_slice(&f.pixels[src..src + s.w as usize]);
            }
        }
        Ok(slots)
    }

    /// Frees a whole page (atlas eviction): all its bytes return to 0, its
    /// slots become invalid (generation bumped) and packing restarts in
    /// it, in shelf order.
    pub fn clear_page(&mut self, page: u32) -> Result<(), AtlasError> {
        let p = self
            .pages
            .get_mut(page as usize)
            .ok_or(AtlasError::NoPage(page))?;
        p.pixels.fill(0);
        p.dirty = true;
        p.generation += 1;
        self.layouts[page as usize] = Layout::new();
        Ok(())
    }

    /// The bytes of `slot`, row-major (what a draw through the slot sees).
    pub fn read(&self, slot: AtlasSlot) -> Result<Vec<u8>, AtlasError> {
        if slot.is_empty() {
            return Ok(Vec::new());
        }
        let page = self
            .pages
            .get(slot.page as usize)
            .ok_or(AtlasError::NoPage(slot.page))?;
        let mut out = Vec::with_capacity((slot.w * slot.h) as usize);
        for row in 0..slot.h {
            let at = ((slot.y + row) * PAGE_SIZE + slot.x) as usize;
            out.extend_from_slice(&page.pixels[at..at + slot.w as usize]);
        }
        Ok(out)
    }

    /// Compares each slot with the frame put there and its 1-pixel ring
    /// with index 0. Every pixel is counted once per slot it belongs to.
    ///
    /// The atlas's own self-test: it proves both that the copy is exact
    /// and that the gutter really stayed transparent.
    pub fn check<'a>(
        &self,
        placed: impl IntoIterator<Item = (AtlasSlot, &'a super::IndexFrame)>,
    ) -> Result<CheckReport, AtlasError> {
        let mut r = CheckReport::default();
        for (s, f) in placed {
            if s.is_empty() {
                if !f.is_empty() {
                    r.frame_mismatches += f.pixels.len() as u64;
                }
                continue;
            }
            let page = self
                .pages
                .get(s.page as usize)
                .ok_or(AtlasError::NoPage(s.page))?;
            if (s.w, s.h) != (f.width, f.height) {
                r.frame_mismatches += f.pixels.len() as u64;
                r.first.get_or_insert((s.page, s.x, s.y));
                continue;
            }
            // Ring: rows y-1 and y+h over x-1..=x+w, columns x-1 and x+w
            // over y..y+h. GUTTER ≥ 1 keeps these inside the page.
            let (x0, y0, x1, y1) = (s.x - 1, s.y - 1, s.x + s.w, s.y + s.h);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let v = page.pixels[(y * PAGE_SIZE + x) as usize];
                    let inside = x >= s.x && x < x1 && y >= s.y && y < y1;
                    let want = if inside {
                        f.pixels[((y - s.y) * s.w + (x - s.x)) as usize]
                    } else {
                        0
                    };
                    if v != want {
                        if inside {
                            r.frame_mismatches += 1;
                        } else {
                            r.gutter_mismatches += 1;
                        }
                        r.first.get_or_insert((s.page, x, y));
                    }
                }
            }
        }
        Ok(r)
    }

    /// Pages changed since the last call, in page order; marks them clean.
    pub fn take_dirty(&mut self) -> Vec<u32> {
        let mut out = Vec::new();
        for (i, p) in self.pages.iter_mut().enumerate() {
            if p.dirty {
                p.dirty = false;
                out.push(i as u32);
            }
        }
        out
    }

    /// Test-only access for perturbation tests (M08).
    #[cfg(test)]
    pub(crate) fn page_pixels_mut(&mut self, page: u32) -> &mut [u8] {
        &mut self.pages[page as usize].pixels
    }
}
