/// Deviations of ±0.5 (curvature − 0.5) stored as 16-bit integers.
pub const DEVIATION_SCALE: f32 = i16::MAX as f32 * 2.0;

/// Curvature blurred at one detail size: deviations from flat, stored `step` times coarser than the map.
pub struct CurvatureSizeLevel {
    pub step: u32,
    pub width: u32,
    pub height: u32,
    /// UV tile dimensions in stored texels; interpolation must stay inside one tile
    pub block: (u32, u32),
    pub deviation: Vec<i16>,
}

impl CurvatureSizeLevel {
    /// The deviation at map texel `(x, y)`, interpolated within its UV tile
    pub fn sample(&self, x: u32, y: u32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }

        let step = self.step.max(1);
        let axis = |texel: u32, block: u32, length: u32| {
            let block = block.max(1);
            let first = if block >= length { 0 } else { (texel / block.saturating_mul(step) * block).min(length - 1) };
            let last = first.saturating_add(block - 1).min(length - 1);
            let position = ((texel as f32 + 0.5) / step as f32 - 0.5).clamp(first as f32, last as f32);
            let lower = position.floor() as u32;
            (lower, (lower + 1).min(last), position - lower as f32)
        };
        let (x0, x1, blend_x) = axis(x, self.block.0, self.width);
        let (y0, y1, blend_y) = axis(y, self.block.1, self.height);
        let stored = |x: u32, y: u32| self.deviation.get((y * self.width + x) as usize).map_or(0.0, |value| f32::from(*value) / DEVIATION_SCALE);
        let top = stored(x0, y0) + (stored(x1, y0) - stored(x0, y0)) * blend_x;
        let bottom = stored(x0, y1) + (stored(x1, y1) - stored(x0, y1)) * blend_x;
        top + (bottom - top) * blend_y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_blend_the_stored_texels() {
        let level = CurvatureSizeLevel { step: 2, width: 2, height: 1, block: (2, 1), deviation: vec![0, (0.25 * DEVIATION_SCALE) as i16] };
        assert!(level.sample(0, 0).abs() < 1e-4, "the first stored texel covers map texels 0 and 1");
        assert!((level.sample(3, 0) - 0.25).abs() < 1e-3);
        assert!(level.sample(2, 0) > 0.0 && level.sample(2, 0) < 0.25, "between stored texels the value blends");
    }
}
