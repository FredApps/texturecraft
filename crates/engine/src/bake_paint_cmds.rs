use crate::commands::{CommandSpec, always, bad, boolp, f32p, strp, u64p};
use crate::{Result, Session, cmd};
use serde_json::Value;
use std::sync::Arc;
use texturecraft_bake::BakePaintMap;
use texturecraft_core::BakePaintTarget;
use texturecraft_geom::glam::Vec2;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("bake.skew.set", "Set Skew Correction", [], None, "{ts?, value?: 0..1 | path: grayscale image}", always, |session, parameters| set(
            session,
            parameters,
            BakePaintTarget::Skew
        )),
        cmd!(
            "bake.skew.paint",
            "Paint Skew Correction",
            [],
            None,
            "{ts?, points: [[u,v],...], radius?: 0.03, value?: 1, hardness?: 0.5, strength?: 1, size?: 1024, continue?: false}",
            always,
            |session, parameters| paint(session, parameters, BakePaintTarget::Skew)
        ),
        cmd!("bake.skew.clear", "Remove Skew Correction", [], None, "{ts?}", always, |session, parameters| clear(session, parameters, BakePaintTarget::Skew)),
        cmd!(
            "bake.cage.paint",
            "Paint Cage",
            [],
            None,
            "{ts?, points: [[u,v],...], radius?: 0.03, value?: 1 (frontal distance multiplier, 0..1), hardness?: 0.5, strength?: 1, size?: 1024, continue?: false}",
            always,
            |session, parameters| paint(session, parameters, BakePaintTarget::Cage)
        ),
        cmd!("bake.cage.reset", "Reset Painted Cage", [], None, "{ts?}", always, |session, parameters| clear(session, parameters, BakePaintTarget::Cage)),
    ]
}

fn texture_set(session: &Session, parameters: &Value) -> Result<usize> {
    let index = match parameters.get("ts") {
        None => session.ts(),
        Some(value) => value.as_u64().and_then(|value| usize::try_from(value).ok()).ok_or_else(|| bad("bake.paint", "ts must be a texture set index"))?,
    };
    session.doc.project.texture_set(index)?;
    Ok(index)
}

fn number(parameters: &Value, name: &str, default: f32, minimum: f32, maximum: f32) -> Result<f32> {
    if parameters.get(name).is_none() {
        return Ok(default);
    }
    f32p(parameters, name)
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or_else(|| bad("bake.paint", format!("{name} must be in {minimum}..={maximum}")))
}

fn set(session: &mut Session, parameters: &Value, target: BakePaintTarget) -> Result<Value> {
    let texture_set = texture_set(session, parameters)?;
    let map = match strp(parameters, "path") {
        Some(path) => load_image(path)?,
        None => BakePaintMap::constant(number(parameters, "value", 1.0, 0.0, 1.0)?),
    };
    map.validate().map_err(|error| bad("bake.paint", error.to_string()))?;
    session.edit("Set Bake Paint Map", |session| {
        session.doc.bake_paint.insert((texture_set, target), Arc::new(map));
        Ok(())
    })?;
    Ok(crate::bake_cmds::meshes_json(session))
}

fn clear(session: &mut Session, parameters: &Value, target: BakePaintTarget) -> Result<Value> {
    let texture_set = texture_set(session, parameters)?;
    if session.doc.bake_paint.contains_key(&(texture_set, target)) {
        session.edit("Clear Bake Paint Map", |session| {
            session.doc.bake_paint.remove(&(texture_set, target));
            Ok(())
        })?;
    }
    Ok(crate::bake_cmds::meshes_json(session))
}

fn paint(session: &mut Session, parameters: &Value, target: BakePaintTarget) -> Result<Value> {
    const MAX_POINTS: usize = 100_000;
    let texture_set = texture_set(session, parameters)?;
    let points = parameters
        .get("points")
        .and_then(Value::as_array)
        .filter(|points| !points.is_empty() && points.len() <= MAX_POINTS)
        .ok_or_else(|| bad("bake.paint", "points must contain 1 to 100000 UV coordinates"))?;
    let points = points
        .iter()
        .map(|point| {
            let coordinate = |index| point.get(index).and_then(Value::as_f64).map(|value| value as f32).filter(|value| value.is_finite());
            match (coordinate(0), coordinate(1)) {
                (Some(u), Some(v)) => Ok(Vec2::new(u, v)),
                _ => Err(bad("bake.paint", "each point must contain two finite UV coordinates")),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let value = number(parameters, "value", 1.0, 0.0, 1.0)?;
    let radius = number(parameters, "radius", 0.03, 0.00001, 1.0)?;
    let hardness = number(parameters, "hardness", 0.5, 0.0, 1.0)?;
    let strength = number(parameters, "strength", 1.0, 0.0, 1.0)?;
    let size = match parameters.get("size") {
        None => 1024,
        Some(_) => u64p(parameters, "size").ok_or_else(|| bad("bake.paint", "size must be an integer"))?,
    };
    if !(16..=4096).contains(&size) {
        return Err(bad("bake.paint", "size must be between 16 and 4096"));
    }
    let default = match target {
        BakePaintTarget::Cage => 1.0,
        BakePaintTarget::Skew => 0.0,
    };
    let label = match target {
        BakePaintTarget::Cage => "Paint Cage",
        BakePaintTarget::Skew => "Paint Skew Correction",
    };
    session.edit_merging(label, boolp(parameters, "continue") == Some(true), |session| {
        let map = session.doc.bake_paint.entry((texture_set, target)).or_insert_with(|| Arc::new(BakePaintMap::constant(default)));
        if map.values.len() == 1 {
            *map = Arc::new(map.resized(size as u32, size as u32).map_err(|error| bad("bake.paint", error.to_string()))?);
        }
        Arc::make_mut(map).paint(&points, radius, value, hardness, strength).map_err(|error| bad("bake.paint", error.to_string()))
    })?;
    Ok(crate::bake_cmds::meshes_json(session))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_image(path: &str) -> Result<BakePaintMap> {
    let length = std::fs::metadata(path).map_err(|error| bad("bake.paint", error.to_string()))?.len();
    if length > 256 << 20 {
        return Err(bad("bake.paint", "image exceeds 256 MiB"));
    }
    let bytes = std::fs::read(path).map_err(|error| bad("bake.paint", error.to_string()))?;
    let image = texturecraft_formats::decode_image_within(&bytes, texturecraft_bake::MAX_TEXELS).map_err(|error| bad("bake.paint", error.to_string()))?;
    Ok(BakePaintMap {
        width: image.width,
        height: image.height,
        values: image.rgba.iter().map(|pixel| ((pixel[0] + pixel[1] + pixel[2]) / 3.0).clamp(0.0, 1.0)).collect(),
    })
}

#[cfg(target_arch = "wasm32")]
fn load_image(_: &str) -> Result<BakePaintMap> {
    Err(bad("bake.paint", "file paths are unavailable in the browser"))
}
