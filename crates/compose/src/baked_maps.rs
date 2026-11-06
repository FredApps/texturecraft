use std::collections::BTreeMap;

use texturecraft_core::{BakedMap, BlockScales};

/// Dense mesh maps with `components()` floats per texel in row-major order
/// Each UV tile fills its block independently of the working resolution
/// The width is `resolution`; height follows the output size and UV tile grid
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedMaps {
    /// Width in texels
    pub resolution: u32,
    pub maps: BTreeMap<BakedMap, std::sync::Arc<Vec<f32>>>,
}

impl BakedMaps {
    /// Sample a working atlas texel without resampling the stored bake
    pub fn sample_working(&self, map: BakedMap, x: u32, y: u32, size: (u32, u32), scales: &BlockScales) -> Option<[f32; 3]> {
        let uv = [(x as f32 + 0.5) / size.0.max(1) as f32, (y as f32 + 0.5) / size.1.max(1) as f32];
        self.sample_uv(map, scales.bake_uv(uv, size)?)
    }

    /// Nearest sample in the uniform bake atlas
    pub fn sample_uv(&self, map: BakedMap, uv: [f32; 2]) -> Option<[f32; 3]> {
        if uv.iter().any(|value| !value.is_finite()) {
            return None;
        }

        let size = (self.resolution, self.height(map));
        self.sample(map, (uv[0] * size.0 as f32) as u32, (uv[1] * size.1 as f32) as u32, size)
    }

    /// Height in texels of map `m`, from its length
    pub fn height(&self, m: BakedMap) -> u32 {
        self.maps.get(&m).map_or(0, |d| (d.len() / (self.resolution.max(1) as usize * m.components())) as u32)
    }

    /// Sample the corresponding texel of an image with the supplied output size
    pub fn sample(&self, map: BakedMap, x: u32, y: u32, output_size: (u32, u32)) -> Option<[f32; 3]> {
        let data = self.maps.get(&map)?;
        let width = u64::from(self.resolution.max(1));
        let height = u64::from(self.height(map).max(1));
        let sample_x = (u64::from(x) * width / u64::from(output_size.0.max(1))).min(width - 1);
        let sample_y = (u64::from(y) * height / u64::from(output_size.1.max(1))).min(height - 1);
        let component_count = map.components();
        let offset = ((sample_y * width + sample_x) as usize).checked_mul(component_count)?;
        let texel = data.get(offset..offset.checked_add(component_count)?)?;
        Some(if component_count == 3 { [texel[0], texel[1], texel[2]] } else { [texel[0]; 3] })
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32, size: (u32, u32)) -> Option<f32> {
        self.sample(m, x, y, size).map(|v| v[0])
    }
}
