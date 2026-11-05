/// Divergence of the tangent-space normal map; positive curvature is convex
pub(crate) fn curvature_from_normal(
    normal: &[f32],
    covered: &[bool],
    width: usize,
    tile_size: (usize, usize),
    settings: texturecraft_core::NormalMapCurvatureSettings,
    output: &mut [f32],
) {
    use rayon::prelude::*;
    let radius_texels = (tile_size.0.min(tile_size.1) / 512).max(1);
    let strength = 2.0f32.powf((settings.details - 0.5) * 4.0);
    let normal_component = |x: usize, y: usize, component: usize, fallback: f32| match covered.get(y * width + x) {
        Some(true) => normal.get((y * width + x) * 3 + component).copied().unwrap_or(fallback),
        _ => fallback,
    };
    output.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            if covered.get(y * width + x) != Some(&true) {
                continue;
            }
            let (center_x, center_y) = (normal_component(x, y, 0, 0.5), normal_component(x, y, 1, 0.5));
            let (left, right) = (
                if x % tile_size.0 >= radius_texels { normal_component(x - radius_texels, y, 0, center_x) } else { center_x },
                if x % tile_size.0 + radius_texels < tile_size.0 && x + radius_texels < width {
                    normal_component(x + radius_texels, y, 0, center_x)
                } else {
                    center_x
                },
            );
            let (up, down) = (
                if y % tile_size.1 >= radius_texels { normal_component(x, y - radius_texels, 1, center_y) } else { center_y },
                if y % tile_size.1 + radius_texels < tile_size.1 && y + radius_texels < covered.len() / width.max(1) {
                    normal_component(x, y + radius_texels, 1, center_y)
                } else {
                    center_y
                },
            );
            *value = (0.5 + ((right - left) + (up - down)) * strength).clamp(0.0, 1.0);
        }
    });
}
