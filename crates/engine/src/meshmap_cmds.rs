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
    let img = s.images.get(name).cloned().ok_or_else(|| bad(CMD, format!("no image resource `{name}`")))?;
    let set = s.doc.project.texture_set(ts)?.clone();
    let tiles = set.tile_grid().map(|grid| tile_images(&s.images, name, &set.uv_tiles).map(|imgs| (grid, imgs))).transpose()?;
    // Every mesh map of a set shares one size: the other maps', or what a bake would make (a
    // block of the bake resolution per tile of the atlas).
    let (cols, rows) = tiles.as_ref().map_or((1, 1), |(g, _)| (g.cols, g.rows));
    let (res, height) = match s.doc.baked.get(&ts).filter(|b| !b.maps.is_empty()) {
        Some(b) => (b.resolution, b.maps.keys().next().map_or(b.resolution, |m| b.height(*m))),
        None => {
            let (width, height) = set.bake.common.output_size();
            (width.saturating_mul(cols), height.saturating_mul(rows))
        }
    };
    if res == 0 || height == 0 || u64::from(res) * u64::from(height) > texturecraft_bake::MAX_TEXELS || (tiles.is_none() && res > MAX_RESOLUTION) {
        return Err(bad(CMD, format!("mesh map size {res} × {height} is too large or empty")));
    }
    let directx = map == BakedMap::Normal && s.doc.project.normal_format == NormalFormat::DirectX;
    let data = match &tiles {
        None => resample(map.components(), (res, height), directx, |u, v| img.sample(u, v)),
        // An atlas texel reads the image of the tile its mesh UV falls in.
        Some((grid, imgs)) => resample(map.components(), (res, height), directx, |u, v| {
            let uv = set.from_atlas(grid, [u, v]);
            udim_at(uv).and_then(|t| imgs.get(&t)).map_or([0.0; 4], |img| img.sample(uv[0] - uv[0].floor(), uv[1] - uv[1].floor()))
        }),
    };
    s.edit("Use image as mesh map", |s| {
        let entry = s.doc.baked.entry(ts).or_default();
        entry.resolution = res;
        entry.maps.insert(map, Arc::new(data));
        s.doc.project.texture_set_mut(ts)?.baked.insert(map);
        Ok(())
    })?;
    Ok(json!({ "map": map.id(), "resolution": res }))
}

/// The image of every UV tile of a set: `name` with its tile number (`ao.1001.png`) swapped for
/// each tile's.
fn tile_images(images: &ImageStore, name: &str, tiles: &[u32]) -> Result<BTreeMap<u32, Arc<ImageRes>>> {
    const CMD: &str = "textureSet.meshMap.set";
    let at = udim_in(name).ok_or_else(|| bad(CMD, format!("`{name}` names no UV tile: a UV-tiled set takes one image per tile, named like ao.1001.png")))?;
    tiles
        .iter()
        .map(|&t| {
            let tile_name = format!("{}{t}{}", name.get(..at).unwrap_or_default(), name.get(at + 4..).unwrap_or_default());
            images.get(&tile_name).cloned().map(|img| (t, img)).ok_or_else(|| bad(CMD, format!("no image `{tile_name}` for UV tile {t}")))
        })
        .collect()
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
