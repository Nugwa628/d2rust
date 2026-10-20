// Spec: specs/client/render-pipeline.md (§A2 atlas pages)
//! The Bevy edge of the atlas: each page becomes one `PAGE_SIZE²` R8Uint
//! texture of palette indices (colors are applied later, when drawn).
//!
//! Changed pages are re-uploaded whole; a page keeps its handle for its
//! lifetime, so draws that name a page never change handle. The CPU side
//! lives in [`super::atlas`]; this module only copies dirty pages into
//! Bevy images.

use bevy::asset::InvalidGenerationError;
use bevy::prelude::*;

use super::atlas::{Atlas, PAGE_SIZE};
use crate::render::index_image;

/// Texture handles of the atlas pages, by page number.
#[derive(Resource, Debug, Default, Clone)]
pub struct AtlasTextures {
    /// One image handle per page; a page keeps its handle for life.
    pub pages: Vec<Handle<Image>>,
}

/// Why a page upload failed.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// A page has no texture yet and is not the next page in line, so the
    /// handles would no longer be indexed by page number.
    #[error("atlas page {page} has no texture yet but only {handles} pages do")]
    PageOrder { page: u32, handles: usize },
    /// The page's texture handle no longer refers to a live image.
    #[error("atlas page texture: {0}")]
    Asset(#[from] InvalidGenerationError),
}

/// Uploads every page changed since the last call, in page order.
/// Returns the pages uploaded.
pub fn upload_dirty(
    atlas: &mut Atlas,
    images: &mut Assets<Image>,
    textures: &mut AtlasTextures,
) -> Result<Vec<u32>, UploadError> {
    let dirty = atlas.take_dirty();
    for &p in &dirty {
        let pixels = atlas.pages()[p as usize].pixels.clone();
        let image = index_image(PAGE_SIZE, PAGE_SIZE, pixels);
        match textures.pages.get(p as usize) {
            Some(h) => images.insert(h.id(), image)?,
            // A new page is dirty from birth and pages only grow at the
            // end, so a page without a handle is always the next one.
            None if textures.pages.len() == p as usize => textures.pages.push(images.add(image)),
            None => {
                return Err(UploadError::PageOrder {
                    page: p,
                    handles: textures.pages.len(),
                })
            }
        }
    }
    Ok(dirty)
}
