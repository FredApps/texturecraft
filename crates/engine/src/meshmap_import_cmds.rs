//! Import mesh maps baked in another app: many files at once, each to the set and slot its file name names

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use texturecraft_core::BakedMap;

use crate::commands::{CommandSpec, always, bad, strp};
use crate::meshmap_cmds::{FIRST_TILE, image_map_size, tile_number, use_images};
use crate::meshmap_file_name::{baked_map_from_file_name, texture_set_from_file_name};
use crate::resource_cmds::read_image_file;
use crate::{Result, Session, cmd};

const CMD: &str = "textureSet.meshMap.import";

/// Most files one import takes: every mesh map of dozens of sets, each split over a few UV tiles
const MAX_FILES: usize = 512;

/// Bound retained file bytes, decoded RGBA floats and resampled maps together, before allocating pixels
const MAX_TOTAL_BYTES: u64 = 2 << 30;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "textureSet.meshMap.import",
        "Import baked maps",
        [],
        None,
        "{files: [{path, map?: mesh map id}]} → {assigned: [{path, ts, map}], unmatched: [path]} — each image file becomes the mesh map its name names (`Body_ao.png`, `Body_world_normal.png`; or `map`) of the texture set its name starts with (any name when there is one set), read as by textureSet.meshMap.set. A UV-tiled set takes one file per tile, named with the tile number (`Body_ao.1001.png`, `Body_ao.1002.png`). Left out and listed in `unmatched`: files matching no set or map, a second file for one slot or tile, files of a UV-tiled set without one of its tile numbers, and every file of a map missing a tile. One undo step (none when nothing matches); at most 512 files and 2 GiB of file bytes, decoded images and mesh maps; desktop only",
        always,
        import
    )]
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    import_with_budget(s, p, MAX_TOTAL_BYTES)
}

fn import_with_budget(s: &mut Session, p: &Value, max_bytes: u64) -> Result<Value> {
    let files = p.get("files").and_then(Value::as_array).ok_or_else(|| bad(CMD, "`files` (a list of {path, map?}) is required"))?;
    if files.len() > MAX_FILES {
        return Err(bad(CMD, format!("at most {MAX_FILES} files at once")));
    }

    let set_names: Vec<&str> = s.doc.project.texture_sets.iter().map(|set| set.name.as_str()).collect();
    let mut slots: BTreeMap<(usize, BakedMap), BTreeMap<u32, &str>> = BTreeMap::new();
    let mut unmatched = Vec::new();
    for file in files {
        let path = strp(file, "path").ok_or_else(|| bad(CMD, "every file needs a `path`"))?;
        let given_map = match file.get("map") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(BakedMap::parse(id).ok_or_else(|| bad(CMD, format!("unknown mesh map `{id}`")))?),
            Some(_) => return Err(bad(CMD, "`map` must be a mesh map id string")),
        };

        match slot_of(s, &set_names, path, given_map) {
            Some((slot, tile)) if !slots.get(&slot).is_some_and(|tiles| tiles.contains_key(&tile)) => {
                slots.entry(slot).or_default().insert(tile, path);
            }
            _ => unmatched.push(path),
        }
    }

    // A UV-tiled map needs a file for every tile of its set
    slots.retain(|(ts, _), tiles| {
        let complete =
            s.doc.project.texture_sets.get(*ts).is_some_and(|set| set.tile_grid().is_none() || set.uv_tiles.iter().all(|tile| tiles.contains_key(tile)));
        if !complete {
            unmatched.extend(tiles.values());
        }
        complete
    });
    if slots.is_empty() {
        return Ok(json!({ "assigned": [], "unmatched": unmatched }));
    }

    let result = s.edit("Import baked maps", |s| {
        let mut assigned = Vec::new();
        let mut remaining_bytes = max_bytes;
        for ((ts, map), tiles) in &slots {
            let (width, height) = image_map_size(s, CMD, *ts)?;
            let map_bytes = u64::from(width) * u64::from(height) * map.components() as u64 * size_of::<f32>() as u64;
            reserve_bytes(&mut remaining_bytes, map_bytes)?;

            let mut images = BTreeMap::new();
            for (tile, path) in tiles {
                let bytes = read_image_file(CMD, path, remaining_bytes)?;
                reserve_bytes(&mut remaining_bytes, bytes.len() as u64)?;

                let bytes_per_texel = size_of::<[f32; 4]>() as u64;
                let name = s.add_image_resource(path, bytes, remaining_bytes / bytes_per_texel)?;
                let image = s.images.get(&name).ok_or_else(|| bad(CMD, format!("no decoded image `{name}`")))?;
                reserve_bytes(&mut remaining_bytes, u64::from(image.width) * u64::from(image.height) * bytes_per_texel)?;
                images.insert(*tile, image.clone());
                assigned.push(json!({"path": path, "ts": ts, "map": map.id()}));
            }
            use_images(s, CMD, *ts, *map, &images)?;
        }
        Ok(assigned)
    });
    // A failed edit leaves the document as it was; forget the images it decoded
    if result.is_err() {
        s.sync_images();
    }

    Ok(json!({ "assigned": result?, "unmatched": unmatched }))
}

fn reserve_bytes(remaining_bytes: &mut u64, bytes: u64) -> Result<()> {
    *remaining_bytes = remaining_bytes.checked_sub(bytes).ok_or_else(|| bad(CMD, "the files, decoded images and mesh maps exceed the import memory budget"))?;
    Ok(())
}

/// The texture set and mesh map a file is for, and its UV tile ([`FIRST_TILE`] on a set without UV tiles)
fn slot_of(s: &Session, set_names: &[&str], path: &str, given_map: Option<BakedMap>) -> Option<((usize, BakedMap), u32)> {
    let ts = texture_set_from_file_name(path, set_names)?;
    let map = given_map.or_else(|| baked_map_from_file_name(path, set_names.get(ts).copied().unwrap_or_default()))?;
    let texture_set = s.doc.project.texture_sets.get(ts)?;
    let tile = match texture_set.tile_grid() {
        Some(_) => tile_number(Path::new(path).file_name()?.to_str()?).filter(|tile| texture_set.uv_tiles.contains(tile))?,
        None => FIRST_TILE,
    };
    Some(((ts, map), tile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::BakeResolution;

    #[test]
    fn import_budget_counts_decoded_images_and_resampled_maps() {
        let dir = std::env::temp_dir().join(format!("texturecraft-import-budget-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = [dir.join("ao.png"), dir.join("height.png")];
        for path in &paths {
            image::RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255])).save(path).unwrap();
        }
        let file_bytes: u64 = paths.iter().map(|path| std::fs::metadata(path).unwrap().len()).sum();
        let decoded_bytes = 2 * 16 * 16 * 16;
        let map_bytes = 2 * 16 * 16 * 4;
        let required_bytes = file_bytes + decoded_bytes + map_bytes;
        let params = json!({"files": paths.map(|path| json!({"path": path}))});
        let mut session = Session::new_project("budget", texturecraft_geom::primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
        session.doc.project.texture_sets[0].bake.common.resolution = BakeResolution::R16;
        let revision = session.revision;

        for budget in [file_bytes, file_bytes + decoded_bytes, required_bytes - 1] {
            let result = import_with_budget(&mut session, &params, budget);
            assert!(result.is_err(), "budget {budget} must cover decoded pixels and mesh maps: {result:?}");
            assert!(session.doc.resources.is_empty() && session.images.is_empty());
            assert!(session.doc.baked.values().all(|baked| baked.maps.is_empty()));
            assert_eq!(session.revision, revision);
            assert!(!session.modified);
            assert!(session.undo_label().is_none());
        }

        let result = import_with_budget(&mut session, &params, required_bytes).unwrap();
        assert_eq!(result["assigned"].as_array().unwrap().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
