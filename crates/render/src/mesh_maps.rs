//! Debug textures for the mesh-map view.

use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;
use texturecraft_texel::linear_to_srgb;

/// RGBA8 of baked map `m` resampled to `size × size` stretched over the map (mid gray where not
/// baked). Scalar maps are shown as grayscale; ID maps as their colors. Values are display
/// (sRGB) encoded.
pub fn mesh_map_rgba(baked: Option<&BakedMaps>, m: BakedMap, size: u32) -> Vec<u8> {
    let size = size.max(1);
    let mut out = vec![128u8; (size * size * 4) as usize];
    let Some(b) = baked.filter(|b| b.maps.contains_key(&m)) else { return out };
    for y in 0..size {
        for x in 0..size {
            let v = b.sample(m, x, y, (size, size)).unwrap_or([0.5; 3]);
            let i = ((y * size + x) * 4) as usize;
            for k in 0..3 {
                // Data maps are shown as stored, i.e. treated as display values.
                out[i + k] = (v[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            out[i + 3] = 255;
        }
    }
    let _ = linear_to_srgb;
    out
}
