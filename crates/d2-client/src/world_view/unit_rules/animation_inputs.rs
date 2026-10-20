//! Source pictures for component-local generation. The caller owns the source
//! files; decoded pictures stay in memory or the caller's local cache.

use super::super::unit_assets::{component_codes, unit_cof_with, UnitLooks};
use super::frame_pack::FrameKey;
use super::generation_job::AnimationInput;
use crate::assets::path::{read_cof, read_dc6, read_dcc, FileSource};
use crate::bridge::ClientUnit;
use crate::frames::FrameSet;
use crate::rules::unit_composite::CofPath;
use crate::rules::unit_composite::{
    file_direction, file_format, Code, ComponentCodes, CompositeKind, FileFormat, PlayerLook,
};
use std::collections::BTreeMap;

pub fn component_key(codes: &ComponentCodes, component: u8) -> FrameKey {
    FrameKey {
        kind: match codes.kind {
            CompositeKind::Player => 0,
            CompositeKind::Monster => 1,
            CompositeKind::Object => 2,
        },
        token: codes.unit_token,
        mode: codes.mode_token,
        component,
        component_token: codes.component_token,
        armor_class: codes.armor_class,
        weapon_class: codes.weapon_class,
        direction: 0,
        frame: 0,
        sub_frame: 0,
    }
}

#[derive(Clone)]
pub struct AnimationSelection {
    pub unit: ClientUnit,
    pub hands: Option<Code>,
    pub equipment: Option<PlayerLook>,
    pub directions: Vec<u8>,
}

pub fn load_animation(
    source: &dyn FileSource,
    looks: &UnitLooks,
    selection: &AnimationSelection,
) -> Result<Vec<AnimationInput>, String> {
    let unit = &selection.unit;
    let directions = &selection.directions;
    let name = unit_cof_with(looks, unit, selection.hands).ok_or("animation has no COF name")?;
    let path = name.path().map_err(|e| e.to_string())?;
    let cof = read_cof(source, path.as_str()).ok_or("COF not found")??;
    let mut inputs = Vec::new();
    for layer in &cof.layers {
        let Some(codes) = component_codes(looks, unit, &name, layer, selection.equipment.as_ref())
        else {
            continue;
        };
        let format = file_format(&codes, unit.class, unit.mode as u8);
        let path = codes.path(format).map_err(|e| e.to_string())?;
        let mut pictures = BTreeMap::new();
        match format {
            FileFormat::Dcc => {
                let Some(file) = read_dcc(source, path.as_str()) else {
                    continue;
                };
                let file = file?;
                for &direction in directions {
                    let dir = file_direction(file.directions.len() as u8, direction)
                        .map_err(|e| e.to_string())?;
                    pictures.insert(
                        direction,
                        FrameSet::from_dcc(&file, dir)
                            .map_err(|e| e.to_string())?
                            .frames,
                    );
                }
            }
            FileFormat::Dc6 => {
                let Some(file) = read_dc6(source, path.as_str()) else {
                    continue;
                };
                let file = file?;
                for &direction in directions {
                    let dir = file_direction(file.header.directions as u8, direction)
                        .map_err(|e| e.to_string())?;
                    pictures.insert(
                        direction,
                        FrameSet::from_dc6(&file, dir)
                            .map_err(|e| e.to_string())?
                            .frames,
                    );
                }
            }
        }
        inputs.push(AnimationInput {
            key: component_key(&codes, layer.component),
            directions: pictures,
            looping: true,
        });
    }
    Ok(inputs)
}

/// One component file as a generation input: every source facing of the
/// file, keyed in the client's 64-direction space. The install-time bake and
/// the play loader's staleness check build the same input from the same file.
/// `None` when the source has no such file.
pub fn load_component_file(
    source: &dyn FileSource,
    path: &str,
    format: FileFormat,
    key: FrameKey,
) -> Result<Option<AnimationInput>, String> {
    let mut pictures = BTreeMap::new();
    match format {
        FileFormat::Dcc => {
            let Some(file) = read_dcc(source, path) else {
                return Ok(None);
            };
            let file = file?;
            let count = file.directions.len() as u8;
            for direction in super::animation_bake::source_facings(count) {
                let dir = file_direction(count, direction).map_err(|e| e.to_string())?;
                pictures.insert(
                    direction,
                    FrameSet::from_dcc(&file, dir)
                        .map_err(|e| e.to_string())?
                        .frames,
                );
            }
        }
        FileFormat::Dc6 => {
            let Some(file) = read_dc6(source, path) else {
                return Ok(None);
            };
            let file = file?;
            let count = file.header.directions as u8;
            for direction in super::animation_bake::source_facings(count) {
                let dir = file_direction(count, direction).map_err(|e| e.to_string())?;
                pictures.insert(
                    direction,
                    FrameSet::from_dc6(&file, dir)
                        .map_err(|e| e.to_string())?
                        .frames,
                );
            }
        }
    }
    Ok(Some(AnimationInput {
        key,
        directions: pictures,
        looping: true,
    }))
}
