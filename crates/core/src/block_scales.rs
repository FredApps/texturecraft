/// Per-block UV tile scale for working-to-bake coordinates, filter radii, and height slopes
/// Empty when every tile fills its block; see [`crate::TextureSet::tile_scale`]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockScales {
    pub(crate) block: u32,
    pub(crate) cols: u32,
    pub(crate) scales: Vec<f32>,
}

impl BlockScales {
    /// Working atlas UV → uniform bake atlas UV, excluding unused portions of tile blocks
    pub fn bake_uv(&self, uv: [f32; 2], working_size: (u32, u32)) -> Option<[f32; 2]> {
        if working_size.0 == 0 || working_size.1 == 0 || uv.iter().any(|value| !value.is_finite() || !(0.0..1.0).contains(value)) {
            return None;
        }

        if self.scales.is_empty() {
            return Some(uv);
        }

        let block = self.block.max(1) as f32;
        let position = [uv[0] * working_size.0 as f32 / block, uv[1] * working_size.1 as f32 / block];
        let origin = position.map(f32::floor);
        let scale = self.at((origin[0] * block) as u32, (origin[1] * block) as u32);
        let local = [(position[0] - origin[0]) / scale, (position[1] - origin[1]) / scale];
        if local.iter().any(|value| !value.is_finite() || !(0.0..1.0).contains(value)) {
            return None;
        }

        Some([(origin[0] + local[0]) * block / working_size.0.max(1) as f32, (origin[1] + local[1]) * block / working_size.1.max(1) as f32])
    }

    /// The scale at atlas texel (x, y)
    pub fn at(&self, x: u32, y: u32) -> f32 {
        let (bx, by) = (x / self.block.max(1), y / self.block.max(1));
        if bx >= self.cols {
            return 1.0;
        }

        let index = (by as usize).checked_mul(self.cols as usize).and_then(|index| index.checked_add(bx as usize));
        index.and_then(|index| self.scales.get(index)).copied().unwrap_or(1.0)
    }
}
