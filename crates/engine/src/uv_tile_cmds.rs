//! UV tile (UDIM) layout commands: per-tile resolution and switching a texture set between
//! classic and UV tile mode after creation. Both change where texels sit in the set's atlas, so
//! painted pixels and baked maps are resampled through mesh UV space into the new layout (the
//! session then maps the mesh's UVs into the new atlas, see `Session::follow_uv_layouts`).

use std::sync::Arc;

use rayon::prelude::*;
use serde_json::{Value, json};
use texturecraft_compose::BakedMaps;
use texturecraft_core::{TextureSet, udim_at};
use texturecraft_texel::TileImage;

use crate::commands::{CommandSpec, always, bad, boolp, u64p};
use crate::session::{Document, check_resolution, check_size, detect_uv_tiles};
use crate::set_cmds::ts_param;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "textureSet.tile.resolution",
            "UV tile resolution",
            [],
            None,
            "{ts?, tile: UDIM number, resolution: 128..8192 (power of two; the set's resolution clears the override)} → {ts, tile, resolution, size: [w, h]}",
            always,
            tile_resolution
        ),
        cmd!(
            "textureSet.uvTiles",
            "UV tiles mode",
            [],
            None,
            "{ts?, enabled: bool} → {ts, uvTiles, dropped: [UDIM numbers whose pixels were left out]} (painted pixels are resampled into the new layout)",
            always,
            uv_tiles
        ),
    ]
}

fn tile_resolution(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "textureSet.tile.resolution";
    let texture_set_index = ts_param(session, params, COMMAND)?;
    let tile = u64p(params, "tile").and_then(|tile| u32::try_from(tile).ok()).ok_or_else(|| bad(COMMAND, "`tile` (a UDIM number) is required"))?;
    let resolution = u64p(params, "resolution").ok_or_else(|| bad(COMMAND, "`resolution` is required"))?;
    let resolution = check_resolution(u32::try_from(resolution).unwrap_or(0))?;
    let texture_set = session.doc.project.texture_set(texture_set_index)?;
    if !texture_set.uv_tiles.contains(&tile) {
        return Err(bad(COMMAND, format!("texture set \"{}\" has no UV tile {tile} (tiles: {:?})", texture_set.name, texture_set.uv_tiles)));
    }

    let removes_override = resolution == texture_set.resolution && texture_set.tile_resolutions.contains_key(&tile);
    if texture_set.tile_resolution(tile) == resolution && !removes_override {
        return Ok(json!({ "ts": texture_set_index, "tile": tile, "resolution": resolution, "size": texture_set.size() }));
    }

    session.cancel_stroke();
    let size = session.edit("UV tile resolution", |session| {
        let texture_set = session.doc.project.texture_set_mut(texture_set_index)?;
        let previous = texture_set.clone();
        if resolution == texture_set.resolution {
            texture_set.tile_resolutions.remove(&tile);
        } else {
            texture_set.tile_resolutions.insert(tile, resolution);
        }
        check_size(texture_set)?;
        let size = texture_set.size();
        if previous.tile_resolution(tile) != resolution {
            relayout(&mut session.doc, texture_set_index, &previous, &mut session.log)?;
        }
        Ok(size)
    })?;
    Ok(json!({ "ts": texture_set_index, "tile": tile, "resolution": resolution, "size": size }))
}

fn uv_tiles(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "textureSet.uvTiles";
    let texture_set_index = ts_param(session, params, COMMAND)?;
    let enabled = boolp(params, "enabled").ok_or_else(|| bad(COMMAND, "`enabled` (true or false) is required"))?;
    let texture_set = session.doc.project.texture_set(texture_set_index)?;
    let tiles = if enabled {
        detect_uv_tiles(&session.source, texture_set.material, Some(true), &mut Vec::new())
            .map_err(|error| bad(COMMAND, format!("texture set \"{}\": {error}", texture_set.name)))?
    } else {
        Vec::new()
    };
    if tiles == texture_set.uv_tiles {
        return Ok(json!({ "ts": texture_set_index, "uvTiles": tiles, "dropped": [] }));
    }

    // Leaving UV tile mode keeps tile 1001, the area a classic set paints
    let dropped: Vec<u32> = texture_set.uv_tiles.iter().copied().filter(|tile| !enabled && *tile != 1001).collect();
    session.cancel_stroke();
    session.edit(if enabled { "Use UV tiles" } else { "Stop using UV tiles" }, |session| {
        let texture_set = session.doc.project.texture_set_mut(texture_set_index)?;
        let previous = texture_set.clone();
        texture_set.tile_resolutions.retain(|tile, _| tiles.contains(tile));
        texture_set.bake_excluded_tiles.retain(|tile| tiles.contains(tile));
        texture_set.uv_tiles = tiles.clone();
        check_size(texture_set)?;
        relayout(&mut session.doc, texture_set_index, &previous, &mut session.log)
    })?;
    Ok(json!({ "ts": texture_set_index, "uvTiles": tiles, "dropped": dropped }))
}

/// Mesh UV → normalized atlas position of UV tile layout `set` (None where the layout holds
/// no texel for it): classic sets cover UV 0..1.
fn to_layout(set: &TextureSet, m: [f32; 2]) -> Option<[f32; 2]> {
    match set.tile_grid() {
        Some(g) => udim_at(m).filter(|u| set.uv_tiles.contains(u)).map(|_| set.to_atlas(&g, m, m)),
        None => (m.iter().all(|v| (0.0..=1.0).contains(v))).then_some(m),
    }
}

/// Normalized atlas position of layout `new` → the mesh UV it shows (None in the unused part of
/// a block or a block without a tile).
fn from_layout(set: &TextureSet, a: [f32; 2]) -> Option<[f32; 2]> {
    let Some(g) = set.tile_grid() else { return Some(a) };
    let (bx, by) = ((a[0] * g.cols as f32).floor(), (a[1] * g.rows as f32).floor());
    if bx < 0.0 || by < 0.0 {
        return None;
    }
    let udim = g.udim(bx as u32, by as u32);
    let scale = set.tile_scale(udim);
    let local = (a[0] * g.cols as f32 - bx, a[1] * g.rows as f32 - by);
    (set.uv_tiles.contains(&udim) && local.0 <= scale && local.1 <= scale).then(|| set.from_atlas(&g, a))
}

/// Move painted images into the working layout and baked maps only when the tile grid changes
pub(crate) fn relayout(doc: &mut Document, ts: usize, old: &TextureSet, log: &mut Vec<String>) -> Result<()> {
    let new = doc.project.texture_set(ts)?.clone();
    let map = |uv: [f32; 2]| from_layout(&new, uv).and_then(|mesh_uv| to_layout(old, mesh_uv));
    let (old_size, new_size) = (old.size(), new.size());
    let texel = |x: f32, y: f32| map([x / new_size.0 as f32, y / new_size.1 as f32]).map(|uv| (uv[0] * old_size.0 as f32, uv[1] * old_size.1 as f32));
    let all = TileImage::new(new_size.0, new_size.1, 1);
    let tiles: Vec<(u32, u32)> = (0..all.tiles_y()).flat_map(|ty| (0..all.tiles_x()).map(move |tx| (tx, ty))).collect();

    // Atlas blocks align with the 64-texel storage tiles, making each mapping affine
    crate::layer_ops::map_set_images(doc, ts, |image, empty| remap_image(image, new_size, empty, &texel, &tiles, true));
    remap_bake_paint(doc, ts, old, &new, &map)?;

    if old.uv_tiles != new.uv_tiles
        && let Some(baked) = doc.baked.get(&ts)
    {
        let mut old_bake = old.clone();
        let mut new_bake = new.clone();
        old_bake.tile_resolutions.clear();
        new_bake.tile_resolutions.clear();
        let bake_map = |uv| from_layout(&new_bake, uv).and_then(|mesh_uv| to_layout(&old_bake, mesh_uv));
        match remap_baked(baked, &old_bake, &new_bake, &bake_map) {
            Some(baked) => {
                doc.baked.insert(ts, baked);
            }
            None => {
                doc.baked.remove(&ts);
                doc.project.texture_set_mut(ts)?.baked.clear();
                log.push(format!("Texture set \"{}\": the baked mesh maps no longer fit; bake them again", new.name));
            }
        }
    }
    Ok(())
}

/// `img` resampled to `size` over destination `tiles` (64-texel tiles; the others stay empty)
/// through `map` (destination texel center → source position in texels), tiles in parallel.
/// With `affine` (the map is affine or None over each tile) tiles whose corners read only
/// unallocated texels are skipped unvisited.
pub(crate) fn remap_image(
    img: &TileImage,
    size: (u32, u32),
    empty: &[u16],
    map: &(dyn Fn(f32, f32) -> Option<(f32, f32)> + Sync),
    tiles: &[(u32, u32)],
    affine: bool,
) -> TileImage {
    let mut out = TileImage::new(size.0, size.1, img.comps());
    let skip = |tx: u32, ty: u32| {
        let t = texturecraft_texel::TILE as f32;
        let (x0, y0) = (tx as f32 * t + 0.5, ty as f32 * t + 0.5);
        let (x1, y1) = (((tx as f32 + 1.0) * t).min(size.0 as f32) - 0.5, ((ty as f32 + 1.0) * t).min(size.1 as f32) - 0.5);
        let corners: Option<Vec<(f32, f32)>> = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].iter().map(|(x, y)| map(*x, *y)).collect();
        match corners {
            Some(c) => {
                let (lo, hi) =
                    c.iter().fold(((f32::MAX, f32::MAX), (f32::MIN, f32::MIN)), |(lo, hi), p| ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1))));
                !img.any_allocated_in(lo.0, lo.1, hi.0, hi.1)
            }
            None => [(x0, y0), (x1, y1)].iter().all(|(x, y)| map(*x, *y).is_none()),
        }
    };
    let data: Vec<_> = tiles
        .par_iter()
        .filter(|(tx, ty)| !(affine && skip(*tx, *ty)))
        .filter_map(|&(tx, ty)| Some((tx, ty, img.remapped_tile(size.0, size.1, tx, ty, empty, map)?)))
        .collect();
    for (tx, ty, d) in data {
        out.set_tile(tx, ty, Some(Arc::new(d)));
    }
    out
}

/// Baked maps of layout `old` resampled (nearest texel) into layout `new` at the same density
/// per UV tile; None when that is beyond what bakes may hold.
fn remap_baked(baked: &BakedMaps, old: &TextureSet, new: &TextureSet, map: &(dyn Fn([f32; 2]) -> Option<[f32; 2]> + Sync)) -> Option<BakedMaps> {
    let first_map = baked.maps.keys().next()?;
    let (width, height) = remapped_size((baked.resolution, baked.height(*first_map)), old, new)?;
    let mut output = BakedMaps { resolution: width, ..BakedMaps::default() };
    for (kind, data) in &baked.maps {
        let (component_count, old_width, old_height) = (kind.components(), baked.resolution as usize, baked.height(*kind) as usize);
        let neutral = kind.neutral();
        let mut texels = vec![0.0f32; width as usize * height as usize * component_count];
        texels.par_chunks_mut(width as usize * component_count).enumerate().for_each(|(y, row)| {
            for (x, pixel) in row.chunks_mut(component_count).enumerate() {
                let source = map([(x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32]).and_then(|uv| {
                    let source_x = ((uv[0] * old_width as f32) as usize).min(old_width.saturating_sub(1));
                    let source_y = ((uv[1] * old_height as f32) as usize).min(old_height.saturating_sub(1));
                    let offset = (source_y * old_width + source_x) * component_count;
                    data.get(offset..offset + component_count)
                });
                pixel.copy_from_slice(source.unwrap_or(&neutral[..component_count]));
            }
        });
        output.maps.insert(*kind, Arc::new(texels));
    }
    Some(output)
}

fn remapped_size((width, height): (u32, u32), old: &TextureSet, new: &TextureSet) -> Option<(u32, u32)> {
    let grid = |set: &TextureSet| set.tile_grid().map_or((1, 1), |grid| (grid.cols, grid.rows));
    let ((old_columns, old_rows), (new_columns, new_rows)) = (grid(old), grid(new));
    let width = (width / old_columns.max(1)).max(1).checked_mul(new_columns)?;
    let height = (height / old_rows.max(1)).max(1).checked_mul(new_rows)?;
    (u64::from(width) * u64::from(height) <= texturecraft_bake::MAX_TEXELS).then_some((width, height))
}

fn remap_bake_paint(
    document: &mut Document,
    texture_set: usize,
    old: &TextureSet,
    new: &TextureSet,
    map: &(dyn Fn([f32; 2]) -> Option<[f32; 2]> + Sync),
) -> Result<()> {
    use texturecraft_bake::BakePaintMap;
    use texturecraft_core::BakePaintTarget;
    for ((index, target), painted) in &mut document.bake_paint {
        if *index != texture_set {
            continue;
        }
        let (width, height) = remapped_size((painted.width, painted.height), old, new)
            .ok_or_else(|| bad("textureSet.uvTiles", "painted bake maps would exceed the supported size"))?;
        let empty = match target {
            BakePaintTarget::Cage => 1.0,
            BakePaintTarget::Skew => 0.0,
        };
        let mut values = vec![empty; width as usize * height as usize];
        values.par_chunks_mut(width as usize).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                if let Some(uv) = map([(x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32]) {
                    *value = painted.sample(texturecraft_geom::glam::Vec2::from_array(uv));
                }
            }
        });
        *painted = Arc::new(BakePaintMap { width, height, values });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute;
    use texturecraft_core::Channel;
    use texturecraft_geom::primitives;

    fn udim_session() -> Session {
        let mut s = Session::new_project("u", primitives::builtin("udim_plane").unwrap(), "builtin:udim_plane", None, 256).unwrap();
        assert_eq!(s.doc.project.texture_sets[0].uv_tiles, vec![1001, 1002]);
        s.update();
        s
    }

    /// Paint a constant value over every covered texel of layer `id` (base color).
    fn paint_all(s: &mut Session, value: u16) {
        let id = s.doc.project.texture_sets[0].stack[0];
        let size = s.doc.project.texture_sets[0].size();
        let gbuf = s.caches[0].gbuf.clone();
        let img = s.doc.pixels.layer_mut(id, Channel::BaseColor, size);
        for y in 0..size.1 {
            for x in 0..size.0 {
                if gbuf.covered((y * size.0 + x) as usize) {
                    img.set(x, y, &[value, value, value, 65535]);
                }
            }
        }
    }

    /// Mean painted alpha over the covered texels of the set's current layout.
    fn painted_fraction(s: &Session) -> f64 {
        let id = s.doc.project.texture_sets[0].stack[0];
        let img = s.doc.pixels.layer(id, Channel::BaseColor).unwrap();
        let gbuf = &s.caches[0].gbuf;
        let (mut sum, mut n) = (0.0, 0.0);
        for (i, t) in gbuf.tri.iter().enumerate() {
            if *t != texturecraft_geom::NO_TRI {
                let (x, y) = (i as u32 % gbuf.width, i as u32 / gbuf.width);
                sum += img.get(x, y)[3] as f64 / 65535.0;
                n += 1.0;
            }
        }
        sum / n
    }

    #[test]
    fn per_tile_resolution_moves_tiles_and_keeps_pixels() {
        let mut s = udim_session();
        paint_all(&mut s, 40000);
        let out = execute(&mut s, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
        // Tile 1001 keeps 256, so blocks stay 256 and tile 1002 fills a quarter of its block.
        assert_eq!(out["size"], json!([512, 256]));
        let set = &s.doc.project.texture_sets[0];
        assert_eq!((set.tile_resolution(1001), set.tile_resolution(1002), set.block_resolution()), (256, 128, 256));
        // Mesh UVs of tile 1002 now sit in the top-left quarter of block 1.
        let uv_max = s.mesh.uvs.iter().filter(|u| u.x > 0.5).fold(0.0f32, |m, u| m.max(u.x));
        assert!(uv_max <= 0.75 + 1e-4, "{uv_max}");
        assert!(painted_fraction(&s) > 0.97, "painting survives the relayout");
        let inspect = execute(&mut s, "textureSet.inspect", &json!({})).unwrap();
        assert_eq!(inspect["tiles"][1]["resolution"], 128);
        // Raising one tile above the rest grows every block.
        execute(&mut s, "textureSet.tile.resolution", &json!({"tile": 1001, "resolution": 512})).unwrap();
        assert_eq!(s.doc.project.texture_sets[0].size(), (1024, 512));
        assert!(painted_fraction(&s) > 0.97);
        // Undo restores the layout, the mesh UVs and the pixels.
        s.undo().unwrap();
        s.undo().unwrap();
        assert!(s.doc.project.texture_sets[0].tile_resolutions.is_empty());
        assert_eq!(s.mesh.uvs, Session::new_project("u", primitives::builtin("udim_plane").unwrap(), "x", None, 256).unwrap().mesh.uvs);
        assert!(painted_fraction(&s) > 0.97);
        // Errors: unknown tile, bad resolution, classic set.
        assert!(execute(&mut s, "textureSet.tile.resolution", &json!({"tile": 1005, "resolution": 128})).is_err());
        assert!(execute(&mut s, "textureSet.tile.resolution", &json!({"tile": 1001, "resolution": 100})).is_err());
    }

    #[test]
    fn removing_a_tile_override_at_the_sets_current_resolution_restores_inheritance() {
        let mut session = udim_session();
        execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
        execute(&mut session, "textureSet.resolution", &json!({"resolution": 128})).unwrap();
        let before = session.doc.project.clone();
        let history_steps = session.history_labels().len();

        execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();

        assert!(session.doc.project.texture_sets[0].tile_resolutions.is_empty());
        assert_eq!(session.history_labels().len(), history_steps + 1);
        let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
        assert!(reopened.doc.project.texture_sets[0].tile_resolutions.is_empty());
        session.undo().unwrap();
        assert_eq!(session.doc.project, before);
        session.redo().unwrap();
        execute(&mut session, "textureSet.resolution", &json!({"resolution": 256})).unwrap();
        assert_eq!(session.doc.project.texture_sets[0].tile_resolution(1002), 256);
        assert_eq!(session.doc.project.texture_sets[0].bake, before.texture_sets[0].bake);
    }

    #[test]
    fn export_writes_each_tile_at_its_own_size() {
        let mut s = udim_session();
        paint_all(&mut s, 50000);
        execute(&mut s, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
        let opts = crate::file_cmds::export_options(&s, &json!({"preset": "Document channels + Normal + AO", "format": "png"}), "t").unwrap();
        let files = s.export_files(&[0], &opts).unwrap();
        // PNG: width and height are the big-endian words after the IHDR tag.
        let png_size = |b: &[u8]| (u32::from_be_bytes(b[16..20].try_into().unwrap()), u32::from_be_bytes(b[20..24].try_into().unwrap()));
        assert!(files.len() >= 4);
        for f in &files {
            let want = if f.name.contains("1002") { 128 } else { 256 };
            assert_eq!(png_size(&f.bytes), (want, want), "{}", f.name);
        }
    }

    /// Paint channel `c` of the first layer over every covered texel as `f(u)`, `u` the texel's
    /// position across its own UV tile (0..1), so every tile shows the same pattern in UV space.
    fn paint_across_tiles(s: &mut Session, c: Channel, f: impl Fn(f32) -> Vec<u16>) {
        let set = s.doc.project.texture_sets[0].clone();
        let (grid, (w, h)) = (set.tile_grid().unwrap(), set.size());
        let gbuf = s.caches[0].gbuf.clone();
        let img = s.doc.pixels.layer_mut(set.stack[0], c, (w, h));
        for y in 0..h {
            for x in 0..w {
                if gbuf.covered((y * w + x) as usize) {
                    let m = set.from_atlas(&grid, [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32]);
                    img.set(x, y, &f(m[0].rem_euclid(1.0)));
                }
            }
        }
        s.touched();
        while s.update() {}
    }

    /// A tile at half its block's resolution: (session, its block's left texel, its side).
    fn half_res_tile() -> (Session, u32, u32) {
        let mut s = udim_session();
        execute(&mut s, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
        (s, 256, 128)
    }

    #[test]
    fn filter_radii_follow_each_tiles_resolution() {
        let (mut s, x2, side) = half_res_tile();
        let id = s.doc.project.texture_sets[0].stack[0];
        execute(&mut s, "effect.add.filter", &json!({"layer": id.0, "target": "content", "filter": "blur", "radius": 60})).unwrap();
        // Black on the left half of each tile, white on the right: a blurred edge in the middle.
        paint_across_tiles(&mut s, Channel::BaseColor, |u| {
            let v = if u < 0.5 { 0 } else { 65535 };
            vec![v, v, v, 65535]
        });
        // Texels of the soft edge along a row through the middle of a tile (away from its borders).
        let comp = &s.caches[0].composite;
        let edge = |x0: u32, side: u32| {
            (x0 + side / 8..x0 + side * 7 / 8).filter(|x| (0.05..0.95).contains(&comp.value(Channel::BaseColor, *x, side / 2)[0])).count() as i64
        };
        let (wide, narrow) = (edge(0, 256), edge(x2, side));
        assert!(wide > 8, "the blur softens the edge: {wide}");
        // The same width in UV space: half the texels in the half-resolution tile.
        assert!((2 * narrow - wide).abs() <= 4, "{narrow} texels at 128 vs {wide} at 256");
    }

    #[test]
    fn height_slopes_follow_each_tiles_resolution() {
        let (mut s, _, _) = half_res_tile();
        s.doc.project.texture_sets[0].height_strength = 8.0;
        // The same height ramp across each tile.
        paint_across_tiles(&mut s, Channel::Height, |u| vec![texturecraft_texel::to_u16_signed(0.5 * u), 65535]);
        let opts = crate::file_cmds::export_options(&s, &json!({"preset": "Document channels + Normal + AO", "format": "png"}), "t").unwrap();
        let files = s.export_files(&[0], &opts).unwrap();
        // The red (x) of the exported normal at the middle of a tile.
        let tilt = |tile: &str| {
            let f = files.iter().find(|f| f.name.ends_with(&format!("_normal.{tile}.png"))).expect("a normal map per tile");
            let img = image::load_from_memory(&f.bytes).unwrap().to_rgb8();
            img.get_pixel(img.width() / 2, img.height() / 2)[0] as i32
        };
        let (full, half) = (tilt("1001"), tilt("1002"));
        assert!(full < 110, "the ramp tilts the normal: {full}");
        assert!((full - half).abs() <= 2, "the same slope in UV space: {half} at 128 vs {full} at 256");
    }

    #[test]
    fn convert_to_uv_tiles_and_back() {
        let mut s = Session::new_project("u", primitives::builtin("udim_plane").unwrap(), "builtin:udim_plane", None, 256).unwrap();
        execute(&mut s, "textureSet.uvTiles", &json!({"enabled": false})).unwrap();
        assert!(s.doc.project.texture_sets[0].uv_tiles.is_empty());
        assert_eq!(s.doc.project.texture_sets[0].size(), (256, 256));
        s.update();
        paint_all(&mut s, 30000);
        let out = execute(&mut s, "textureSet.uvTiles", &json!({"enabled": true})).unwrap();
        assert_eq!(out["uvTiles"], json!([1001, 1002]));
        assert_eq!(s.doc.project.texture_sets[0].size(), (512, 256));
        // Tile 1001 keeps the classic set's painting; tile 1002 was never painted.
        let id = s.doc.project.texture_sets[0].stack[0];
        let img = s.doc.pixels.layer(id, Channel::BaseColor).unwrap();
        let gbuf = s.caches[0].gbuf.clone();
        let covered = |x: u32| (0..256).filter(|y| gbuf.covered((y * 512 + x) as usize)).collect::<Vec<_>>();
        let x = (0..256).find(|x| !covered(*x).is_empty()).unwrap();
        assert!(covered(x).iter().all(|y| img.get(x, *y)[3] > 60000));
        let x2 = (256..512).find(|x| !covered(*x).is_empty()).unwrap();
        assert!(covered(x2).iter().all(|y| img.get(x2, *y)[3] == 0));
        let back = execute(&mut s, "textureSet.uvTiles", &json!({"enabled": false})).unwrap();
        assert_eq!(back["dropped"], json!([1002]));
        assert!(painted_fraction(&s) > 0.97);
        assert!(execute(&mut s, "textureSet.uvTiles", &json!({})).is_err());
    }
}

#[test]
fn bake_maps_follow_uv_layout_without_losing_rectangular_dimensions() {
    use texturecraft_core::{BakePaintTarget, BakedMap};
    let mut session = Session::new_project("UV", texturecraft_geom::primitives::builtin("udim_plane").unwrap(), "builtin:udim_plane", None, 256).unwrap();
    crate::execute(&mut session, "bake.settings", &json!({"model": {"common": {"resolution": "R64", "resolution_height": "R128"}}})).unwrap();
    crate::execute(&mut session, "bake.run", &json!({"maps": ["normal"]})).unwrap();
    crate::execute(&mut session, "bake.skew.paint", &json!({"points": [[0.125, 0.25]], "radius": 0.06, "size": 64})).unwrap();
    crate::execute(&mut session, "textureSet.uvTiles", &json!({"enabled": false})).unwrap();
    let baked = &session.doc.baked[&0];
    assert_eq!((baked.resolution, baked.height(BakedMap::Normal)), (64, 128));
    let painted = &session.doc.bake_paint[&(0, BakePaintTarget::Skew)];
    assert!(painted.sample(texturecraft_geom::glam::Vec2::new(0.25, 0.25)) > 0.9);
    assert_eq!(painted.sample(texturecraft_geom::glam::Vec2::new(0.75, 0.75)), 0.0);
}
