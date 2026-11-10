//! The baked curvature map at the detail sizes of curvature generators, built once per map and layout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use rayon::prelude::*;
use texturecraft_core::{BakedMap, BlockScales, CurvatureDetail};
use texturecraft_texel::dilate::{DilationMap, NONE};

use crate::curvature_size_level::{CurvatureSizeLevel, DEVIATION_SCALE};
use crate::eval::{BLUR_BOX_PASSES, Inputs, blur};

/// The curvature map padded past its UV islands and blurred at each size of [`CurvatureDetail::SIZES`] after the first.
pub struct CurvatureSizes {
    /// One per blurred size, built once a generator gives that size a weight above 0.
    pub levels: Vec<Option<Arc<CurvatureSizeLevel>>>,
}

/// The bake map size, its UV tile block and the working texture size a cache entry was built for.
type CacheKey = ((u32, u32), (u32, u32), (u32, u32));

/// Built sizes by the curvature data they come from, their layout and the tiles' scales.
type CacheEntry = (Weak<Vec<f32>>, CacheKey, BlockScales, Arc<CurvatureSizes>);

static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

// Undo snapshots keep old maps alive; their derived levels must not grow with the history
const CACHE_BYTES: usize = 256 << 20;
const CACHE_MAPS: usize = 16;

impl CurvatureSizes {
    /// The sizes of the texture set `inp` composites, holding at least those `needed` marks (one flag per blurred size); None without a baked curvature map or a needed size.
    pub fn of(inp: &Inputs, needed: &[bool]) -> Option<Arc<CurvatureSizes>> {
        let curvature = inp.baked.maps.get(&BakedMap::Curvature)?;
        let (width, height) = (inp.baked.resolution, inp.baked.height(BakedMap::Curvature));
        if !needed.contains(&true) || width == 0 || height == 0 {
            return None;
        }

        let grid = inp.project.texture_set(inp.ts).ok().and_then(|set| set.tile_grid());
        let block = grid.map_or((width, height), |grid| (width / grid.cols.max(1), height / grid.rows.max(1)));
        let key = ((width, height), block, (inp.gbuf.width, inp.gbuf.height));
        let built = cached(curvature, key, &inp.block_scales);
        if let Some(sizes) = built.as_ref().filter(|sizes| sizes.holds(needed)) {
            return Some(sizes.clone());
        }

        // Built without the cache locked: the build runs parallel work, and a thread waiting on it may composite a tile that asks the cache again
        let covered = covered_texels(inp, (width, height));
        let sizes = CurvatureSizes::build(curvature, (width, height), &covered, block, needed, built.as_deref());
        Some(store(curvature, key, &inp.block_scales, sizes))
    }

    /// Whether every size `needed` marks is built.
    fn holds(&self, needed: &[bool]) -> bool {
        needed.iter().zip(&self.levels).all(|(needs, level)| !needs || level.is_some())
    }

    /// Pad and blur each UV tile at the requested sizes, reusing levels already in `built`
    fn build(
        curvature: &[f32],
        (width, height): (u32, u32),
        covered: &[bool],
        block: (u32, u32),
        needed: &[bool],
        built: Option<&CurvatureSizes>,
    ) -> CurvatureSizes {
        let padding = DilationMap::in_rectangular_blocks(covered, width, height, None, block);
        let deviation: Vec<f32> =
            padding.src.par_iter().map(|&source| if source == NONE { 0.0 } else { curvature.get(source as usize).copied().unwrap_or(0.5) - 0.5 }).collect();
        let (block_width, block_height) = (block.0.max(1).min(width), block.1.max(1).min(height));

        let levels = CurvatureDetail::SIZES
            .iter()
            .skip(1)
            .enumerate()
            .map(|(level, size)| {
                if let Some(existing) = built.and_then(|sizes| sizes.levels.get(level)).and_then(Option::clone) {
                    return Some(existing);
                }
                if !needed.get(level).copied().unwrap_or(false) {
                    return None;
                }

                // Sizes are texels at 2K; a blur this wide holds nothing finer than half its radius, so it is stored that much coarser
                let radius = (size * block_width as f32 / 2048.0).round().max(1.0) as u32;
                let mut step = (radius / 2).max(1).next_power_of_two();
                // A coarse cell must not average curvature from different UV tiles
                while !block_width.is_multiple_of(step) || !block_height.is_multiple_of(step) {
                    step /= 2;
                }

                let mut blurred = deviation.clone();
                for block_y in (0..height).step_by(block_height as usize) {
                    for block_x in (0..width).step_by(block_width as usize) {
                        let region = (block_x, block_y, block_width.min(width - block_x), block_height.min(height - block_y));
                        blur_block(&mut blurred, width, region, radius);
                    }
                }
                Some(Arc::new(coarsen(&blurred, (width, height), step, (block_width, block_height))))
            })
            .collect();
        CurvatureSizes { levels }
    }
}

/// The cached sizes of `curvature` for layout `key` and tile `scales`, if any
fn cached(curvature: &Arc<Vec<f32>>, key: CacheKey, scales: &BlockScales) -> Option<Arc<CurvatureSizes>> {
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|(source, ..)| source.strong_count() > 0);
    let index = cache.iter().position(|entry| is_entry_for(entry, curvature, key, scales))?;
    let entry = cache.remove(index);
    let sizes = entry.3.clone();
    cache.push(entry);
    Some(sizes)
}

/// Cache `sizes`, merging any levels another thread stored meanwhile
fn store(curvature: &Arc<Vec<f32>>, key: CacheKey, scales: &BlockScales, sizes: CurvatureSizes) -> Arc<CurvatureSizes> {
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sizes = if let Some((.., cached_sizes)) = cache.iter().find(|entry| is_entry_for(entry, curvature, key, scales)) {
        let levels = cached_sizes.levels.iter().zip(sizes.levels).map(|(cached, built)| cached.clone().or(built)).collect();
        CurvatureSizes { levels }
    } else {
        sizes
    };

    // One layout per curvature map, with the most recently used map at the end
    cache.retain(|(source, ..)| source.strong_count() > 0 && !std::ptr::eq(source.as_ptr(), Arc::as_ptr(curvature)));
    let sizes = Arc::new(sizes);
    cache.push((Arc::downgrade(curvature), key, scales.clone(), sizes.clone()));
    trim_cache(&mut cache, CACHE_BYTES);
    sizes
}

/// Evict the least recently used maps, retaining the newest even if it alone exceeds the budget
fn trim_cache(cache: &mut Vec<CacheEntry>, budget: usize) {
    // Keeping the newest prevents a large atlas from rebuilding its levels for every tile
    let mut bytes = 0usize;
    let mut keep = 0;
    for (.., sizes) in cache.iter().rev() {
        let size = sizes.levels.iter().flatten().map(|level| level.deviation.len().saturating_mul(size_of::<i16>())).fold(0usize, usize::saturating_add);
        bytes = bytes.saturating_add(size);
        if keep > 0 && (bytes > budget || keep == CACHE_MAPS) {
            break;
        }
        keep += 1;
    }
    cache.drain(..cache.len() - keep);
}

/// Whether cache `entry` holds the sizes of `curvature` for layout `key` and tile `scales`.
fn is_entry_for((source, cached_key, cached_scales, _): &CacheEntry, curvature: &Arc<Vec<f32>>, key: CacheKey, scales: &BlockScales) -> bool {
    source.upgrade().is_some_and(|data| Arc::ptr_eq(&data, curvature)) && *cached_key == key && cached_scales == scales
}

/// The bake atlas texels (`width × height`) overlapped by covered working texels of `inp`
fn covered_texels(inp: &Inputs, (width, height): (u32, u32)) -> Vec<bool> {
    let working_size = (inp.gbuf.width, inp.gbuf.height);
    if working_size == (width, height) && inp.block_scales == BlockScales::default() {
        return (0..width * height).into_par_iter().map(|texel| inp.gbuf.covered(texel as usize)).collect();
    }

    let covered: Vec<AtomicBool> = (0..width * height).map(|_| AtomicBool::new(false)).collect();
    (0..working_size.0 * working_size.1).into_par_iter().filter(|texel| inp.gbuf.covered(*texel as usize)).for_each(|texel| {
        let (x, y) = (texel % working_size.0, texel / working_size.0);
        let uv = [(x as f32 + 0.5) / working_size.0 as f32, (y as f32 + 0.5) / working_size.1 as f32];
        if let Some(baked_uv) = inp.block_scales.bake_uv(uv, working_size) {
            // A lower-resolution working texel covers a whole rectangle in the bake, not just its center
            let scale = inp.block_scales.at(x, y);
            let footprint = |center: f32, baked: u32, working: u32| {
                let half_width = 0.5 * baked as f32 / (working as f32 * scale);
                let center = center * baked as f32;
                ((center - half_width).floor().max(0.0) as u32)..((center + half_width).ceil() as u32).min(baked)
            };
            let columns = footprint(baked_uv[0], width, working_size.0);
            for baked_y in footprint(baked_uv[1], height, working_size.1) {
                for baked_x in columns.clone() {
                    if let Some(slot) = covered.get((baked_y * width + baked_x) as usize) {
                        slot.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
    });
    covered.into_iter().map(AtomicBool::into_inner).collect()
}

/// Blur the `(left, top, region_width, region_height)` block of `data` (`width` texels per row) in place, reading nothing outside it.
fn blur_block(data: &mut [f32], width: u32, (left, top, region_width, region_height): (u32, u32, u32, u32), radius: u32) {
    let rows = top..top + region_height;
    let mut block: Vec<f32> =
        rows.flat_map(|y| (left..left + region_width).map(move |x| (y * width + x) as usize)).map(|index| data.get(index).copied().unwrap_or(0.0)).collect();
    blur(&mut block, region_width as usize, region_height as usize, radius as usize);

    // The blur reads zero past the block's edges; dividing by the share of its window inside the block keeps the edges at full strength
    let column_weights = window_inside(region_width as usize, radius as usize);
    let row_weights = window_inside(region_height as usize, radius as usize);
    for (values, row_weight) in block.chunks_mut(region_width as usize).zip(&row_weights) {
        for (value, column_weight) in values.iter_mut().zip(&column_weights) {
            *value /= row_weight * column_weight;
        }
    }

    for (row, values) in block.chunks(region_width as usize).enumerate() {
        let start = ((top + row as u32) * width + left) as usize;
        if let Some(target) = data.get_mut(start..start + values.len()) {
            target.copy_from_slice(values);
        }
    }
}

/// The share of the [`blur`] window at each texel of a line `length` long that falls inside it: 1 from three radii in.
fn window_inside(length: usize, radius: usize) -> Vec<f32> {
    let window = (2 * radius + 1) as f64;
    let mut weights = vec![1.0f64; length];
    for _ in 0..BLUR_BOX_PASSES {
        let running_totals: Vec<f64> = std::iter::once(0.0)
            .chain(weights.iter().scan(0.0, |total, weight| {
                *total += weight;
                Some(*total)
            }))
            .collect();
        let total_at = |end: usize| running_totals.get(end.min(length)).copied().unwrap_or(0.0);
        weights = (0..length).map(|texel| (total_at(texel + radius + 1) - total_at(texel.saturating_sub(radius))) / window).collect();
    }
    weights.into_iter().map(|weight| weight as f32).collect()
}

/// `data` averaged over `step × step` squares, clipping the last cells to the map
fn coarsen(data: &[f32], (width, height): (u32, u32), step: u32, block: (u32, u32)) -> CurvatureSizeLevel {
    let (coarse_width, coarse_height) = (width.div_ceil(step), height.div_ceil(step));
    let deviation = (0..coarse_width * coarse_height)
        .into_par_iter()
        .map(|cell| {
            let (cell_x, cell_y) = (cell % coarse_width, cell / coarse_width);
            let columns = cell_x * step..((cell_x + 1) * step).min(width);
            let rows = cell_y * step..((cell_y + 1) * step).min(height);
            let count = (columns.len() * rows.len()).max(1) as f32;
            let texels = rows.flat_map(|y| columns.clone().map(move |x| (y * width + x) as usize));
            let mean = texels.map(|index| data.get(index).copied().unwrap_or(0.0)).sum::<f32>() / count;
            (mean * DEVIATION_SCALE).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect();
    CurvatureSizeLevel { step, width: coarse_width, height: coarse_height, block: (block.0.div_ceil(step), block.1.div_ceil(step)), deviation }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_sizes_do_not_blend_between_uv_tiles() {
        let (width, height) = (512, 512);
        let block = (256, 256);
        let curvature: Vec<f32> = (0..width * height).map(|texel| if (texel % width) / block.0 == (texel / width) / block.1 { 0.8 } else { 0.2 }).collect();
        let covered = vec![true; curvature.len()];
        let sizes = CurvatureSizes::build(&curvature, (width, height), &covered, block, &[true; CurvatureDetail::BLURRED_SIZES], None);

        for level in sizes.levels.iter().flatten() {
            for (x, y) in [(255, 128), (256, 128), (128, 255), (128, 256), (255, 255), (256, 256)] {
                let expected = curvature[(y * width + x) as usize] - 0.5;
                assert!((level.sample(x, y) - expected).abs() < 0.001, "step {} at ({x}, {y}): {} instead of {expected}", level.step, level.sample(x, y));
            }
        }
    }

    #[test]
    fn coverage_preserves_bake_texels_above_the_working_resolution() {
        use texturecraft_core::{Project, TextureSet};
        use texturecraft_geom::{UvGBuffer, primitives};

        let mesh = primitives::plane(2.0, 1);
        let mut project = Project::new("Coverage", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let gbuf = UvGBuffer::new(&mesh, 0, 64, 64);
        let pixels = Default::default();
        let images = Default::default();
        let baked = Default::default();
        let inputs = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: BlockScales::default(),
            preview: None,
            images: &images,
            bounds: mesh.bounds(),
        };

        for size in [(32, 32), (64, 64), (128, 128), (128, 32)] {
            let coverage = covered_texels(&inputs, size);
            assert!(coverage.iter().all(|covered| *covered), "the plane covers the entire {size:?} bake, including between working texel centers");
        }
    }

    #[test]
    fn coverage_expands_scaled_tiles_without_crossing_block_borders() {
        use texturecraft_core::{Project, TextureSet};
        use texturecraft_geom::{NO_TRI, UvGBuffer, primitives};

        let mesh = primitives::plane(2.0, 1);
        let mut project = Project::new("Tile coverage", "builtin:plane");
        let mut set = TextureSet::new("Tiles", 0, 64);
        set.uv_tiles = vec![1001, 1002, 1003];
        set.tile_resolutions.insert(1002, 32);
        let scales = set.block_scales();
        project.texture_sets.push(set);
        let mut gbuf = UvGBuffer::new(&mesh, 0, 192, 64);
        for (index, triangle) in gbuf.tri.iter_mut().enumerate() {
            if !(64..96).contains(&(index % 192)) || index / 192 >= 32 {
                *triangle = NO_TRI;
            }
        }
        let pixels = Default::default();
        let images = Default::default();
        let baked = Default::default();
        let inputs = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: scales,
            preview: None,
            images: &images,
            bounds: mesh.bounds(),
        };

        let covered = covered_texels(&inputs, (384, 128));
        for (index, covered) in covered.iter().enumerate() {
            assert_eq!(*covered, (128..256).contains(&(index % 384)), "bake texel ({}, {})", index % 384, index / 384);
        }
    }

    #[test]
    fn unused_maps_are_evicted_even_while_undo_keeps_the_sources_alive() {
        let sources: Vec<_> = (0..=CACHE_MAPS).map(|_| Arc::new(vec![0.5; 16])).collect();
        let scales = BlockScales::default();
        let key = ((4, 4), (4, 4), (4, 4));
        let sizes = store(&sources[0], key, &scales, CurvatureSizes { levels: Vec::new() });
        let first = Arc::downgrade(&sizes);
        drop(sizes);
        for source in sources.iter().skip(1) {
            store(source, key, &scales, CurvatureSizes { levels: Vec::new() });
        }

        assert!(first.upgrade().is_none(), "undo history may retain the bake, but not an unlimited number of derived maps");
    }

    #[test]
    fn cache_budget_evicts_old_levels_but_keeps_one_large_map() {
        let source = Arc::new(vec![0.5]);
        let key = ((1, 1), (1, 1), (1, 1));
        let entry = || {
            let level = CurvatureSizeLevel { step: 1, width: 4, height: 1, block: (4, 1), deviation: vec![0; 4] };
            let sizes = CurvatureSizes { levels: vec![Some(Arc::new(level))] };
            (Arc::downgrade(&source), key, BlockScales::default(), Arc::new(sizes))
        };
        let mut cache = vec![entry(), entry(), entry()];
        let oldest = Arc::downgrade(&cache[0].3);
        let newest = cache[2].3.clone();

        trim_cache(&mut cache, 16);
        assert_eq!(cache.len(), 2);
        assert!(oldest.upgrade().is_none());
        trim_cache(&mut cache, 1);
        assert_eq!(cache.len(), 1, "the active map must not rebuild once per composite tile");
        assert!(Arc::ptr_eq(&cache[0].3, &newest));
    }

    #[test]
    fn padding_keeps_an_island_border_from_reading_flat() {
        let map_size: u32 = 64;
        let left_half = |texel: u32| texel % map_size < map_size / 2;
        let curvature: Vec<f32> = (0..map_size * map_size).map(|texel| if left_half(texel) { 1.0 } else { 0.0 }).collect();
        let covered: Vec<bool> = (0..map_size * map_size).map(left_half).collect();
        let shape = (map_size, map_size);
        let sizes = CurvatureSizes::build(&curvature, shape, &covered, shape, &[true; CurvatureDetail::BLURRED_SIZES], None);

        // The last covered column, halfway down
        let (border_x, border_y) = (map_size / 2 - 1, map_size / 2);
        for level in sizes.levels.iter().flatten() {
            assert!((level.sample(border_x, border_y) - 0.5).abs() < 0.01, "the border keeps the island's curvature, not the empty texels beside it");
        }
    }

    #[test]
    fn uniform_curvature_keeps_its_strength_at_block_borders() {
        let (map_width, map_height): (u32, u32) = (256, 256);
        // Curvature 0.8 reads 0.3 above the neutral 0.5 at every size
        const CURVATURE: f32 = 0.8;
        let expected = CURVATURE - 0.5;
        let curvature = vec![CURVATURE; (map_width * map_height) as usize];
        let covered = vec![true; curvature.len()];
        let block = (map_width / 2, map_height);
        let sizes = CurvatureSizes::build(&curvature, (map_width, map_height), &covered, block, &[true; CurvatureDetail::BLURRED_SIZES], None);

        // The map's corner, then the last texel of the left block and the first of the right one, halfway down
        let border_texels = [(0, 0), (block.0 - 1, map_height / 2), (block.0, map_height / 2)];
        for level in sizes.levels.iter().flatten() {
            for (x, y) in border_texels {
                assert!((level.sample(x, y) - expected).abs() < 0.01, "texel ({x}, {y}) reads {} instead of the uniform {expected}", level.sample(x, y));
            }
        }
    }

    #[test]
    fn a_new_layout_replaces_the_sizes_of_the_old_one() {
        let curvature = Arc::new(vec![0.5; 16]);
        let scales = BlockScales::default();
        let entries_for_curvature = || {
            let cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.iter().filter(|(source, ..)| std::ptr::eq(source.as_ptr(), Arc::as_ptr(&curvature))).count()
        };

        store(&curvature, ((4, 4), (4, 4), (4, 4)), &scales, CurvatureSizes { levels: Vec::new() });
        store(&curvature, ((4, 4), (2, 2), (8, 8)), &scales, CurvatureSizes { levels: Vec::new() });
        assert_eq!(entries_for_curvature(), 1, "only the latest layout of a curvature map stays cached");
    }

    #[test]
    fn sizes_built_on_demand_match_a_full_build() {
        let map_size: u32 = 64;
        let curvature: Vec<f32> =
            (0..map_size * map_size).map(|texel| 0.5 + 0.4 * ((texel % map_size) as f32 * 0.3).sin() * ((texel / map_size) as f32 * 0.2).cos()).collect();
        let covered = vec![true; (map_size * map_size) as usize];
        let shape = (map_size, map_size);
        let all = vec![true; CurvatureDetail::BLURRED_SIZES];
        let full = CurvatureSizes::build(&curvature, shape, &covered, shape, &all, None);

        let even: Vec<bool> = (0..all.len()).map(|level| level % 2 == 0).collect();
        let partial = CurvatureSizes::build(&curvature, shape, &covered, shape, &even, None);
        assert!(partial.levels.iter().zip(&even).all(|(level, needed)| level.is_some() == *needed), "only the needed sizes are built");

        let completed = CurvatureSizes::build(&curvature, shape, &covered, shape, &all, Some(&partial));
        for ((built, expected), earlier) in completed.levels.iter().zip(&full.levels).zip(&partial.levels) {
            assert_eq!(built.as_ref().map(|level| &level.deviation), expected.as_ref().map(|level| &level.deviation), "the same values either way");
            if let (Some(built), Some(earlier)) = (built, earlier) {
                assert!(Arc::ptr_eq(built, earlier), "sizes already built are kept, not rebuilt");
            }
        }
    }
}
