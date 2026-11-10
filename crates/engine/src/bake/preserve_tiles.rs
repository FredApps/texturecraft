use texturecraft_bake::BakeOutput;
use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;

use crate::{EngineError, Result};

/// Retain excluded tiles from the worker's snapshot, resampling when output size changes
pub(super) fn restore(output: &mut BakeOutput, previous: &BakedMaps, excluded_tiles: &[(u32, u32)], grid: (u32, u32)) -> Result<()> {
    if excluded_tiles.is_empty() {
        return Ok(());
    }

    let size = (output.resolution, output.height);
    let tile_width = size.0 / grid.0.max(1);
    let tile_height = size.1 / grid.1.max(1);
    for (map, pixels) in &mut output.maps {
        let components = map.components();
        for &(column, row) in excluded_tiles {
            for y in row * tile_height..(row + 1) * tile_height {
                let first = (y as usize * size.0 as usize + column as usize * tile_width as usize) * components;
                let last = first + tile_width as usize * components;
                let row_pixels = pixels.get_mut(first..last).ok_or_else(|| EngineError::Other("excluded UV tile lies outside the baked pixels".into()))?;
                for (offset, pixel) in row_pixels.chunks_exact_mut(components).enumerate() {
                    let value = previous.sample(*map, column * tile_width + offset as u32, y, size).unwrap_or_else(|| map.neutral());
                    for (component, value) in pixel.iter_mut().zip(value) {
                        *component = value;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Retain only maps outside the full task selection, once per completed atlas.
/// A one-map streaming pass must not resample maps that are still scheduled to bake.
pub(super) fn restore_unchecked(output: &mut BakeOutput, previous: &BakedMaps, excluded_tiles: &[(u32, u32)], selected: &[BakedMap]) -> Result<()> {
    if excluded_tiles.is_empty() {
        return Ok(());
    }
    let size = (output.resolution, output.height);
    for map in previous.maps.keys() {
        if selected.contains(map) || output.maps.contains_key(map) || (previous.resolution == size.0 && previous.height(*map) == size.1) {
            continue;
        }

        let components = map.components();
        let count = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .and_then(|count| count.checked_mul(components))
            .ok_or_else(|| EngineError::Other("resized bake map exceeds the addressable size".into()))?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(count).map_err(|error| EngineError::Other(format!("cannot retain mesh map {}: {error}", map.id())))?;
        #[cfg(test)]
        RETAINED_ALLOCATIONS.with(|allocations| allocations.borrow_mut().push(*map));
        for y in 0..size.1 {
            for x in 0..size.0 {
                let value = previous.sample(*map, x, y, size).unwrap_or_else(|| map.neutral());
                pixels.extend(value.into_iter().take(components));
            }
        }
        output.maps.insert(*map, pixels);
    }
    Ok(())
}

// Restoration is sequential on the calling thread. Record successful retained-buffer allocations
// only in unit tests, without a global allocator or interference from parallel test threads.
#[cfg(test)]
std::thread_local! {
    static RETAINED_ALLOCATIONS: std::cell::RefCell<Vec<texturecraft_core::BakedMap>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn take_retained_allocations() -> Vec<texturecraft_core::BakedMap> {
    RETAINED_ALLOCATIONS.with(|allocations| std::mem::take(&mut *allocations.borrow_mut()))
}
