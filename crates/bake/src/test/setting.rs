use super::*;
use texturecraft_core::{
    AmbientOcclusionAttenuation, HeightNormalization, IdColorSource, MeshMatchMode, PositionAxis, PositionNormalizationScale, ThicknessNormalization,
};

#[test]
fn rectangular_bakes_and_diffusion_preserve_surface_pixels() {
    let mut mesh = quad();
    for uv in &mut mesh.uvs {
        uv.x *= 0.5;
    }
    let mut parameters = settings(64, &[BakedMap::Position]);
    parameters.common.resolution_height = Some(BakeResolution::R32);
    let plain = run(&mesh, &parameters).unwrap();
    assert_eq!((plain.resolution, plain.height), (64, 32));
    assert_eq!(plain.maps[&BakedMap::Position].len(), 64 * 32 * 3);
    parameters.common.apply_diffusion = true;
    let diffused = run(&mesh, &parameters).unwrap();
    for row in 0..32 {
        let covered = row * 64 * 3..(row * 64 + 32) * 3;
        assert_eq!(plain.maps[&BakedMap::Position][covered.clone()], diffused.maps[&BakedMap::Position][covered]);
    }
    let background = (20 * 64 + 50) * 3;
    assert_eq!(plain.maps[&BakedMap::Position][background], 0.0);
    assert!(diffused.maps[&BakedMap::Position][background] > 0.0);
}

#[test]
fn matching_skips_a_closer_unrelated_high_mesh_and_backfaces() {
    let mut low = quad();
    low.submeshes[0].name = "Panel_low".into();
    let mut high = quad();
    high.submeshes[0].name = "Panel_high".into();
    high.positions.iter_mut().for_each(|position| position.y = 0.02);
    let mut unrelated = quad();
    unrelated.submeshes[0].name = "Other_high".into();
    unrelated.positions.iter_mut().for_each(|position| position.y = 0.04);
    high.append(&unrelated);
    let mut parameters = settings(16, &[BakedMap::Height]);
    parameters.cage.max_frontal = 0.1;
    parameters.cage.relative_to_bounding_box = false;
    parameters.maps.height.parameters.normalization = HeightNormalization::Manual;
    let height = |parameters: &BakeSettings, high: &Mesh| transfer(&low, high, None, parameters).unwrap().maps[&BakedMap::Height][128];
    assert!((height(&parameters, &high) - 0.54).abs() < 1e-5);
    parameters.high_poly.match_mode = MeshMatchMode::ByMeshName;
    assert!((height(&parameters, &high) - 0.52).abs() < 1e-5);
    for triangle in high.indices.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
    assert_eq!(height(&parameters, &high), 0.5);
    parameters.high_poly.ignore_backface = false;
    assert!((height(&parameters, &high) - 0.52).abs() < 1e-5);
}

#[test]
fn occlusion_honors_minimum_distance_units_attenuation_and_named_backfaces() {
    let mut mesh = quad();
    let mut ceiling = quad();
    ceiling.positions.iter_mut().for_each(|position| position.y = 0.1);
    ceiling.submeshes[0].name = "Roof_ignorebf".into();
    mesh.append(&ceiling);
    let mut parameters = settings(16, &[BakedMap::AmbientOcclusion]);
    parameters.maps.ao.parameters.rays =
        RaySettings { rays: 1, spread: 0.0, min_distance: 0.0, max_distance: 1.0, relative_to_bounding_box: false, ..Default::default() };
    let value = |parameters: &BakeSettings| run(&mesh, parameters).unwrap().maps[&BakedMap::AmbientOcclusion][128];
    parameters.maps.ao.parameters.attenuation = AmbientOcclusionAttenuation::None;
    assert_eq!(value(&parameters), 0.0);
    parameters.maps.ao.parameters.attenuation = AmbientOcclusionAttenuation::Linear;
    assert!((value(&parameters) - 0.1).abs() < 1e-4);
    parameters.maps.ao.parameters.attenuation = AmbientOcclusionAttenuation::Smooth;
    assert!((value(&parameters) - 0.028).abs() < 1e-4);
    parameters.maps.ao.parameters.rays.min_distance = 0.15;
    assert_eq!(value(&parameters), 1.0);
    parameters.maps.ao.parameters.rays.min_distance = 0.0;
    parameters.maps.ao.parameters.rays.max_distance = 0.075;
    assert_eq!(value(&parameters), 1.0);
    parameters.maps.ao.parameters.rays.relative_to_bounding_box = true;
    assert!(value(&parameters) < 0.8);
    parameters.maps.ao.parameters.ignore_backface = BackfaceMode::ByMeshName;
    assert_eq!(value(&parameters), 1.0);
    parameters.maps.ao.parameters.ignore_backface = BackfaceMode::Never;
    parameters.maps.ao.parameters.self_occlusion = SelfOcclusion::SameMeshName;
    assert_eq!(value(&parameters), 1.0);
}

#[test]
fn bent_normal_settings_are_independent_of_ambient_occlusion() {
    let mesh = primitives::material_ball();
    let mut parameters = settings(32, &[BakedMap::BentNormal, BakedMap::AmbientOcclusion]);
    let original = run(&mesh, &parameters).unwrap();
    parameters.maps.ao.parameters.ignore_backface = BackfaceMode::Always;
    parameters.maps.ao.parameters.self_occlusion = SelfOcclusion::SameMeshName;
    parameters.maps.ao.parameters.attenuation = AmbientOcclusionAttenuation::None;
    let changed = run(&mesh, &parameters).unwrap();
    assert_eq!(original.maps[&BakedMap::BentNormal], changed.maps[&BakedMap::BentNormal]);
}

#[test]
fn id_sources_keep_material_color_separate_from_vertex_color_and_textures() {
    let low = quad();
    let mut high = quad();
    high.colors = vec![Vec4::new(0.8, 0.1, 0.3, 1.0); high.vertex_count()];
    let colors = vec![Some(Arc::new(BaseColor { factor: Vec4::new(0.2, 0.4, 0.7, 1.0), width: 1, height: 1, texels: vec![[255, 0, 0, 255]] }))];
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high));
    let mut parameters = settings(16, &[BakedMap::Id]);
    for (source, expected) in [(IdColorSource::MaterialColor, [0.2, 0.4, 0.7]), (IdColorSource::VertexColor, [0.8, 0.1, 0.3])] {
        parameters.maps.id.parameters.source = source;
        let request = BakeRequest {
            low_base_color: &[],
            mesh: &low,
            bvh: &low_bvh,
            material: 0,
            settings: &parameters,
            high: Some(HighPoly { mesh: &high, bvh: &high_bvh, base_color: &colors, file_ids: &[0] }),
            cage: None,
            skew: None,
            cage_paint: None,
            tiles: (1, 1),
        };
        let output = bake(&request, &Progress::new()).unwrap();
        for (value, expected) in output.maps[&BakedMap::Id][128 * 3..129 * 3].iter().zip(expected) {
            assert!((value - expected).abs() < 1e-6);
        }
    }
}

#[test]
fn painted_cage_limits_frontal_projection() {
    let low = quad();
    let mut high = quad();
    high.positions.iter_mut().for_each(|position| position.y = 0.08);
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high));
    let mut parameters = settings(16, &[BakedMap::Opacity]);
    parameters.cage.max_frontal = 0.1;
    parameters.cage.relative_to_bounding_box = false;
    let reduced = BakePaintMap::constant(0.5);
    let mut request = BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &low_bvh,
        material: 0,
        settings: &parameters,
        high: Some(HighPoly { mesh: &high, bvh: &high_bvh, base_color: &[], file_ids: &[] }),
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    assert_eq!(bake(&request, &Progress::new()).unwrap().maps[&BakedMap::Opacity][128], 1.0);
    request.cage_paint = Some(&reduced);
    assert_eq!(bake(&request, &Progress::new()).unwrap().maps[&BakedMap::Opacity][128], 0.0);
}

#[test]
fn position_axis_and_material_bounds_change_only_the_requested_space() {
    let mut mesh = quad();
    let mut distant = quad();
    distant.positions.iter_mut().for_each(|position| position.x += 10.0);
    mesh.append(&distant);
    let mut parameters = settings(16, &[BakedMap::Position]);
    parameters.maps.position.parameters.axis = PositionAxis::X;
    let global = run(&mesh, &parameters).unwrap();
    parameters.maps.position.parameters.scale = PositionNormalizationScale::PerMaterial;
    let local = run(&mesh, &parameters).unwrap();
    assert!(global.maps[&BakedMap::Position].iter().all(|value| *value < 0.2));
    assert!(local.maps[&BakedMap::Position].iter().any(|value| *value > 0.9));
    assert!(local.maps[&BakedMap::Position].chunks_exact(3).all(|pixel| pixel[0] == pixel[1] && pixel[1] == pixel[2]));
}

#[test]
fn curvature_radius_and_manual_bounds_control_edge_response() {
    let mesh = primitives::cube(2.0);
    let mut parameters = settings(64, &[BakedMap::Curvature]);
    let narrow = run(&mesh, &parameters).unwrap();
    parameters.maps.curvature.parameters.mesh.sampling_radius *= 4.0;
    let wide = run(&mesh, &parameters).unwrap();
    let edge_count = |output: &BakeOutput| covered_values(&mesh, output, BakedMap::Curvature).iter().filter(|(_, value)| *value > 0.6).count();
    assert!(edge_count(&wide) > edge_count(&narrow));
    parameters.maps.curvature.parameters.mesh.value_min = -2.0;
    parameters.maps.curvature.parameters.mesh.value_max = 2.0;
    let reduced = run(&mesh, &parameters).unwrap();
    assert!(reduced.maps[&BakedMap::Curvature].iter().all(|value| (0.25..=0.75).contains(value)));
}

#[test]
fn thickness_can_preserve_distance_or_normalize_each_tile() {
    let mesh = primitives::rounded_box(Vec3::new(1.0, 0.05, 1.0), 0.0);
    let mut parameters = settings(32, &[BakedMap::Thickness]);
    parameters.maps.thickness.parameters.normalization = ThicknessNormalization::None;
    parameters.maps.thickness.parameters.rays.relative_to_bounding_box = false;
    parameters.maps.thickness.parameters.rays.max_distance = 0.3;
    let distance = run(&mesh, &parameters).unwrap();
    assert!(covered_values(&mesh, &distance, BakedMap::Thickness).iter().all(|(_, value)| *value <= 0.30001));
    parameters.maps.thickness.parameters.normalization = ThicknessNormalization::RelativeToMinMax;
    let normalized = run(&mesh, &parameters).unwrap();
    let values = covered_values(&mesh, &normalized, BakedMap::Thickness);
    assert!(values.iter().any(|(_, value)| *value == 0.0));
    assert!(values.iter().any(|(_, value)| *value == 1.0));
}
