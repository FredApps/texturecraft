//! Debug textures for the mesh-map view.

use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakedMap, BlockScales};

/// RGBA8 of baked map `m` resampled to `size × size` stretched over the map (mid gray where not
/// baked). Scalar maps are shown as grayscale; ID maps as their colors. Values are display
/// (sRGB) encoded.
pub fn mesh_map_rgba(baked: Option<&BakedMaps>, m: BakedMap, size: u32, working_size: (u32, u32), scales: &BlockScales) -> Vec<u8> {
    let size = size.max(1);
    let mut out = vec![128u8; (size * size * 4) as usize];
    let Some(b) = baked.filter(|b| b.maps.contains_key(&m)) else { return out };

    for y in 0..size {
        for x in 0..size {
            let uv = [(x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32];
            let v = scales.bake_uv(uv, working_size).and_then(|uv| b.sample_uv(m, uv)).unwrap_or([0.5; 3]);
            let i = ((y * size + x) * 4) as usize;
            for k in 0..3 {
                // Data maps are shown as stored
                out[i + k] = (v[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use texturecraft_core::TextureSet;

    #[test]
    fn mesh_map_preview_places_bake_detail_in_the_working_tile() {
        let mut set = TextureSet::new("Tiles", 0, 256);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 128);
        let pixels = (0..128).flat_map(|y| (0..256).map(move |x| ((x % 128) + y) as f32 / 255.0)).collect();
        let baked = BakedMaps { resolution: 256, maps: [(BakedMap::AmbientOcclusion, Arc::new(pixels))].into() };
        let preview = mesh_map_rgba(Some(&baked), BakedMap::AmbientOcclusion, 512, set.size(), &set.block_scales());
        assert_eq!(preview[(240 * 512 + 376) * 4], 240);
        assert_eq!(preview[(400 * 512 + 500) * 4], 128, "unused working block area stays neutral");
    }
}
