use super::{bumped, covered_values, quad, quad_texel, settings, smooth_shaded_box, transfer};
use texturecraft_core::{BakeSettings, BakedMap, CageSettings, CommonBakeSettings, CurvatureMethod, CurvatureSettings};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::primitives;

fn normal_map_settings(resolution: u32) -> BakeSettings {
    let mut base = settings(resolution, &[BakedMap::Curvature]);
    base.cage = CageSettings { max_frontal: 0.06, max_rear: 0.02, ..base.cage };
    base.common = CommonBakeSettings { dilation: 4, ..base.common };
    base.maps.curvature.parameters = CurvatureSettings { method: CurvatureMethod::NormalMap, ..base.maps.curvature.parameters };
    base.maps.curvature.parameters.normal_map.enable_seams = false;
    base
}

fn coordinates_beyond(position: Vec3, threshold: f32) -> usize {
    position.abs().to_array().iter().filter(|coordinate| **coordinate > threshold).count()
}

fn lowest_and_highest(values: impl IntoIterator<Item = f32>) -> (f32, f32) {
    values.into_iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lowest, highest), value| (lowest.min(value), highest.max(value)))
}

#[test]
fn curvature_from_the_normal_map_sees_the_bump() {
    let out = transfer(&quad(), &bumped(0.1, 0.12), None, &normal_map_settings(64)).unwrap();
    assert!(!out.maps.contains_key(&BakedMap::WorldNormal), "the helper world normal map is not returned");

    let curvature = &out.maps[&BakedMap::Curvature];
    assert!(curvature[quad_texel(64, 0.0, 0.0)] > 0.55, "convex top {}", curvature[quad_texel(64, 0.0, 0.0)]);
    assert!((curvature[quad_texel(64, 0.9, 0.9)] - 0.5).abs() < 0.02, "flat corner {}", curvature[quad_texel(64, 0.9, 0.9)]);
}

#[test]
fn flat_faces_of_a_smooth_shaded_low_poly_stay_mid_grey() {
    let low = smooth_shaded_box();
    let curvature = covered_values(&low, &transfer(&low, &primitives::cube(2.0), None, &normal_map_settings(256)).unwrap(), BakedMap::Curvature);
    let faces: Vec<f32> = curvature.iter().filter(|(position, _)| coordinates_beyond(*position, 0.7) == 1).map(|(_, value)| *value).collect();

    assert!(!faces.is_empty());
    assert!(faces.iter().all(|value| (value - 0.5).abs() < 0.02), "face range {:?}", lowest_and_highest(faces.iter().copied()));
}

#[test]
fn a_rounded_edge_is_one_convex_band() {
    let bevel = 0.1;
    let low = smooth_shaded_box();
    let curvature =
        covered_values(&low, &transfer(&low, &primitives::rounded_box(Vec3::ONE, bevel), None, &normal_map_settings(512)).unwrap(), BakedMap::Curvature);
    let across_one_edge = |from: f32, to: f32| {
        curvature
            .iter()
            .filter(move |(position, _)| (position.x - 1.0).abs() < 1e-3 && position.z.abs() < 0.3 && (from..to).contains(&position.y))
            .map(|(_, value)| *value)
    };

    let inside_the_bevel = across_one_edge(1.0 - bevel * 0.8, 0.99).fold(1.0f32, f32::min);
    let next_to_the_bevel = lowest_and_highest(across_one_edge(0.5, 1.0 - bevel * 1.5));
    assert!(inside_the_bevel > 0.55, "the band dips inside the bevel: {inside_the_bevel}");
    assert!(next_to_the_bevel.0 > 0.48 && next_to_the_bevel.1 < 0.52, "next to the bevel {next_to_the_bevel:?}");
}

#[test]
fn a_small_bump_has_no_ring_beyond_its_skirt() {
    let (resolution, sigma) = (512, 0.08);
    let out = transfer(&quad(), &bumped(0.08, sigma), None, &normal_map_settings(resolution)).unwrap();
    let curvature = &out.maps[&BakedMap::Curvature];
    let ring: Vec<f32> = (0..64)
        .map(|step| step as f32 / 64.0 * std::f32::consts::TAU)
        .flat_map(|angle| [3.0f32, 4.0].map(|radius| curvature[quad_texel(resolution, radius * sigma * angle.cos(), radius * sigma * angle.sin())]))
        .collect();

    assert!(curvature[quad_texel(resolution, 0.0, 0.0)] > 0.6, "convex top {}", curvature[quad_texel(resolution, 0.0, 0.0)]);
    assert!(ring.iter().all(|value| (value - 0.5).abs() < 0.02), "ring {:?}", lowest_and_highest(ring.iter().copied()));
}

#[test]
fn island_borders_of_flat_faces_do_not_streak() {
    let cube = primitives::cube(2.0);
    let curvature = covered_values(&cube, &transfer(&cube, &cube, None, &normal_map_settings(128)).unwrap(), BakedMap::Curvature);
    let faces: Vec<f32> = curvature.iter().filter(|(position, _)| coordinates_beyond(*position, 0.97) == 1).map(|(_, value)| *value).collect();

    assert!(!faces.is_empty());
    assert!(faces.iter().all(|value| (value - 0.5).abs() < 0.01), "face range {:?}", lowest_and_highest(faces.iter().copied()));
}
