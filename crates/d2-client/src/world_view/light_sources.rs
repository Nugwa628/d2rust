//! The level ambient rows the play preview's light map reads:
//! each level's `leveldefs` `Intensity`, `Red`,
//! `Green`, `Blue`.
//!
//! The light records themselves are model state: the client's
//! kept list `ClientWorld::lights`, created by the model's unit code
//! (`bridge::msg::lighting`: players, monsters, objects) and run per
//! drawn frame by [`super::preview_light::PreviewLight::refresh`].

/// The level ambient columns of the user's tables.
#[derive(Debug, Clone, Default)]
pub struct LightRows {
    /// By level id: `Intensity`, `Red`, `Green`, `Blue` of the leveldefs.
    pub levels: Vec<(u8, u8, u8, u8)>,
}

impl LightRows {
    /// These rows with the level ambients of `defs`.
    pub fn with_levels(mut self, defs: &[d2_data::tables::Leveldefs]) -> Self {
        self.levels = defs
            .iter()
            .map(|d| (d.intensity, d.red, d.green, d.blue))
            .collect();
        self
    }
}

/// The level ambient rows of the user's tables.
pub fn load(archives: &dyn d2_data::bin::TableFiles) -> Result<LightRows, String> {
    use d2_data::tables::{decode_all, Leveldefs};
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let defs: Vec<Leveldefs> = match set.table("leveldefs") {
        Some(t) => decode_all(t).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    Ok(LightRows::default().with_levels(&defs))
}
