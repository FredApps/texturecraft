//! Mesh map slots of the Texture Set Settings: use an image resource as a mesh map (baked in
//! another app, or edited by hand) instead of baking it, or clear a slot.

use std::collections::BTreeMap;
use std::sync::Arc;

use rayon::prelude::*;
use serde_json::{Value, json};
use texturecraft_compose::{ImageRes, ImageStore};
use texturecraft_core::{BakedMap, MAX_RESOLUTION, NormalFormat, UDIM_RANGE, udim_at};

use crate::commands::{CommandSpec, always, bad, strp};
use crate::set_cmds::ts_param;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "textureSet.meshMap.set",
            "Use image as mesh map",
            [],
            None,
            "{ts?, map: mesh map id (normal, ambientOcclusion, curvature, id, height…), image: resource name} → {map, resolution} — resampled to the size of the set's other mesh maps (or its bake size); a normal map is read in the project's normal format. A UV-tiled set takes one image per tile, named alike with the tile number (`image: \"ao.1001.png\"` finds ao.1002.png…)",
            always,
            set
        ),
        cmd!("textureSet.meshMap.clear", "Clear mesh map", [], None, "{ts?, map}", always, clear),
    ]
}

fn map_param(p: &Value, cmd: &str) -> Result<BakedMap> {
    let id = strp(p, "map").ok_or_else(|| bad(cmd, "`map` is required"))?;
    BakedMap::parse(id).ok_or_else(|| bad(cmd, format!("unknown mesh map `{id}`")))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "textureSet.meshMap.set";
    let ts = ts_param(s, p, CMD)?;
    let map = map_param(p, CMD)?;
    let name = strp(p, "image").ok_or_else(|| bad(CMD, "`image` (an image resource name) is required"))?;
    let image = s.images.get(name).cloned().ok_or_else(|| bad(CMD, format!("no image resource `{name}`")))?;
    let texture_set = s.doc.project.texture_set(ts)?;
    let images = match texture_set.tile_grid() {
        Some(_) => tile_images(&s.images, CMD, name, &texture_set.uv_tiles)?,
        None => BTreeMap::from([(FIRST_TILE, image)]),
    };

    let resolution = s.edit("Use image as mesh map", |s| use_images(s, CMD, ts, map, &images))?;
    Ok(json!({ "map": map.id(), "resolution": resolution }))
}

/// The tile number a set without UV tiles keys its one image by
pub(crate) const FIRST_TILE: u32 = *UDIM_RANGE.start();

/// Use `images` (by UV tile; one at [`FIRST_TILE`] without UV tiles) as mesh map `map` of set `ts` in the caller's edit; returns its width
pub(crate) fn use_images(s: &mut Session, cmd: &str, ts: usize, map: BakedMap, images: &BTreeMap<u32, Arc<ImageRes>>) -> Result<u32> {
    let texture_set = s.doc.project.texture_set(ts)?;
    let grid = texture_set.tile_grid();
    if let Some(tile) = texture_set.uv_tiles.iter().find(|tile| grid.is_some() && !images.contains_key(tile)) {
        return Err(bad(cmd, format!("no image for UV tile {tile}")));
    }

    let (width, height) = image_map_size(s, cmd, ts)?;

    let flip_green = map == BakedMap::Normal && s.doc.project.normal_format == NormalFormat::DirectX;
    let data = match (grid, images.values().next()) {
        (_, None) => return Err(bad(cmd, "no image to use")),
        (None, Some(image)) => resample(map.components(), (width, height), flip_green, |u, v| image.sample(u, v)),
        // An atlas texel reads the image of the tile its mesh UV falls in
        (Some(grid), Some(_)) => resample(map.components(), (width, height), flip_green, |u, v| {
            let uv = grid.from_atlas([u, v]);
            udim_at(uv).and_then(|tile| images.get(&tile)).map_or([0.0; 4], |image| image.sample(uv[0] - uv[0].floor(), uv[1] - uv[1].floor()))
        }),
    };

    let entry = s.doc.baked.entry(ts).or_default();
    entry.resolution = width;
    entry.maps.insert(map, Arc::new(data));
    s.doc.project.texture_set_mut(ts)?.baked.insert(map);
    Ok(width)
}

/// Dimensions shared by imported mesh maps, checked before allocating the resampled pixels
pub(crate) fn image_map_size(s: &Session, cmd: &str, ts: usize) -> Result<(u32, u32)> {
    let texture_set = s.doc.project.texture_set(ts)?;
    let grid = texture_set.tile_grid();

    // Every mesh map of a set shares one size: the other maps', or a bake's (a bake-size block per atlas tile)
    let (columns, rows) = grid.map_or((1, 1), |grid| (grid.cols, grid.rows));
    let (width, height) = match s.doc.baked.get(&ts).filter(|baked| !baked.maps.is_empty()) {
        Some(baked) => (baked.resolution, baked.maps.keys().next().map_or(baked.resolution, |other| baked.height(*other))),
        None => {
            let (width, height) = texture_set.bake.common.output_size();
            (width.saturating_mul(columns), height.saturating_mul(rows))
        }
    };
    let empty = width == 0 || height == 0;
    let too_many_texels = u64::from(width) * u64::from(height) > texturecraft_bake::MAX_TEXELS;
    let too_wide = grid.is_none() && width > MAX_RESOLUTION;
    if empty || too_many_texels || too_wide {
        return Err(bad(cmd, format!("mesh map size {width} × {height} is too large or empty")));
    }

    Ok((width, height))
}

/// The image of every UV tile of a set: `name` with its tile number (`ao.1001.png`) swapped for each tile's
fn tile_images(images: &ImageStore, cmd: &str, name: &str, tiles: &[u32]) -> Result<BTreeMap<u32, Arc<ImageRes>>> {
    let tile_at =
        udim_in(name).ok_or_else(|| bad(cmd, format!("`{name}` names no UV tile: a UV-tiled set takes one image per tile, named like ao.1001.png")))?;
    let (before_tile, after_tile) = (name.get(..tile_at).unwrap_or_default(), name.get(tile_at + 4..).unwrap_or_default());
    tiles
        .iter()
        .map(|&tile| {
            let tile_name = format!("{before_tile}{tile}{after_tile}");
            images.get(&tile_name).cloned().map(|image| (tile, image)).ok_or_else(|| bad(cmd, format!("no image `{tile_name}` for UV tile {tile}")))
        })
        .collect()
}

/// The UV tile number in a file name (`Body_ao.1002.png` → 1002)
pub(crate) fn tile_number(file_name: &str) -> Option<u32> {
    udim_in(file_name).and_then(|at| file_name.get(at..at + 4)).and_then(|digits| digits.parse().ok())
}

/// Where the last UDIM tile number (four digits alone, in [`UDIM_RANGE`]) in `name` starts.
fn udim_in(name: &str) -> Option<usize> {
    let b = name.as_bytes();
    let digit = |i: Option<usize>| i.and_then(|i| b.get(i)).is_some_and(u8::is_ascii_digit);
    (0..b.len().saturating_sub(3)).rev().find(|&i| {
        !digit(i.checked_sub(1)) && !digit(Some(i + 4)) && name.get(i..i + 4).and_then(|d| d.parse::<u32>().ok()).is_some_and(|t| UDIM_RANGE.contains(&t))
    })
}

/// `w` × `h` texels of `comps` values each from `sample` (atlas UV → RGBA), as stored in the
/// file (mesh maps hold data, not colors, so nothing is linearized); `flip_green` reads a
/// DirectX normal map.
fn resample(comps: usize, (w, h): (u32, u32), flip_green: bool, sample: impl Fn(f32, f32) -> [f32; 4] + Sync) -> Vec<f32> {
    let mut out = vec![0.0; w as usize * h as usize * comps];
    out.par_chunks_mut(w as usize * comps).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) / h as f32;
        for (x, texel) in row.chunks_exact_mut(comps).enumerate() {
            let mut c = sample((x as f32 + 0.5) / w as f32, v);
            if flip_green {
                c[1] = 1.0 - c[1];
            }
            for (o, c) in texel.iter_mut().zip(c) {
                *o = c;
            }
        }
    });
    out
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "textureSet.meshMap.clear";
    let ts = ts_param(s, p, CMD)?;
    let map = map_param(p, CMD)?;
    if !s.doc.baked.get(&ts).is_some_and(|b| b.maps.contains_key(&map)) {
        return Err(bad(CMD, format!("texture set {ts} has no {} map", map.id())));
    }
    s.edit("Clear mesh map", |s| {
        if let Some(b) = s.doc.baked.get_mut(&ts) {
            b.maps.remove(&map);
        }
        s.doc.project.texture_set_mut(ts)?.baked.remove(&map);
        Ok(())
    })?;
    Ok(Value::Null)
}
