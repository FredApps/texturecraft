use rayon::prelude::*;
use std::collections::BTreeMap;
use texturecraft_core::{BakeSettings, BakedMap, CurvatureMethod, HeightNormalization, ThicknessNormalization};

pub(crate) fn normalize(
    maps: &mut BTreeMap<BakedMap, Vec<f32>>,
    covered: &[bool],
    width: usize,
    tile_size: (usize, usize),
    settings: &BakeSettings,
    mesh_scale: f32,
) {
    for (map, pixels) in maps {
        if !matches!(map, BakedMap::Curvature | BakedMap::Height | BakedMap::Thickness) {
            continue;
        }
        if *map == BakedMap::Height && settings.maps.height.parameters.normalization == HeightNormalization::RelativeToRayDistances {
            continue;
        }
        if *map == BakedMap::Curvature && settings.maps.curvature.parameters.method != CurvatureMethod::Mesh {
            continue;
        }
        pixels.par_chunks_mut(width * tile_size.1).enumerate().for_each(|(tile_row, rows)| {
            let coverage = covered.get(tile_row * width * tile_size.1..).unwrap_or_default();
            for start_x in (0..width).step_by(tile_size.0) {
                let end_x = (start_x + tile_size.0).min(width);
                let mut minimum = f32::INFINITY;
                let mut maximum = f32::NEG_INFINITY;
                for (row_index, row) in rows.chunks(width).enumerate() {
                    for (x, value) in row.iter().enumerate().take(end_x).skip(start_x) {
                        if coverage.get(row_index * width + x) == Some(&true) && value.is_finite() {
                            minimum = minimum.min(*value);
                            maximum = maximum.max(*value);
                        }
                    }
                }
                if !minimum.is_finite() {
                    continue;
                }
                let (lower, upper, flat, raw) = match map {
                    BakedMap::Thickness => {
                        let parameters = settings.maps.thickness.parameters;
                        match parameters.normalization {
                            ThicknessNormalization::RelativeToMinMax => (minimum, maximum, 1.0, false),
                            ThicknessNormalization::RelativeToRayDistances => {
                                let units = if parameters.rays.relative_to_bounding_box { mesh_scale } else { 1.0 };
                                (parameters.rays.min_distance * units, parameters.rays.max_distance * units, 1.0, false)
                            }
                            ThicknessNormalization::None => (0.0, 1.0, 1.0, true),
                        }
                    }
                    BakedMap::Height => match settings.maps.height.parameters.normalization {
                        HeightNormalization::RelativeToRayDistances => {
                            let units = if settings.cage.relative_to_bounding_box { mesh_scale } else { 1.0 };
                            (-settings.cage.max_rear * units, settings.cage.max_frontal * units, 0.5, false)
                        }
                        HeightNormalization::RelativeToMinMax => (minimum, maximum, 0.5, false),
                        HeightNormalization::RelativeToLowMesh => {
                            let magnitude = minimum.abs().max(maximum.abs());
                            (-magnitude, magnitude, 0.5, false)
                        }
                        HeightNormalization::Manual => {
                            let half_range = settings.maps.height.parameters.scaling_divisor * 0.5;
                            (-half_range, half_range, 0.5, false)
                        }
                    },
                    _ => {
                        let parameters = settings.maps.curvature.parameters.mesh;
                        if parameters.auto_tonemapping {
                            let magnitude = minimum.abs().max(maximum.abs());
                            (-magnitude, magnitude, 0.5, false)
                        } else {
                            (parameters.value_min, parameters.value_max, 0.5, false)
                        }
                    }
                };
                if raw {
                    continue;
                }
                let range = upper - lower;
                for (row_index, row) in rows.chunks_mut(width).enumerate() {
                    for (x, value) in row.iter_mut().enumerate().take(end_x).skip(start_x) {
                        if coverage.get(row_index * width + x) == Some(&true) {
                            *value = if range > f32::EPSILON { ((*value - lower) / range).clamp(0.0, 1.0) } else { flat };
                        }
                    }
                }
            }
        });
    }
}
