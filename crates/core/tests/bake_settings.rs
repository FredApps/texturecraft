#![recursion_limit = "256"]

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use texturecraft_core::{self as bake, BakedMap, Project, TextureSet};

#[test]
fn reference_defaults_cover_common_and_per_map_settings() {
    let rays = json!({
        "rays": 64, "min_distance": 0.00001, "max_distance": 1.0,
        "relative_to_bounding_box": true, "spread": 180.0, "distribution": "Cosine"
    });
    let mut thickness_rays = rays.clone();
    thickness_rays["max_distance"] = json!(0.1);
    let mut expected = json!({
        "common": {"is_enabled": true, "resolution": "R2048", "resolution_height": null, "antialiasing": "None", "dilation": 1, "apply_diffusion": true},
        "high_poly": {"match_mode": "Always", "use_low_as_high": false, "ignore_backface": true, "low_suffix": "_low", "high_suffix": "_high", "ignore_backface_suffix": "_ignorebf"},
        "cage": {"kind": "DistanceBased", "max_frontal": 0.01, "max_rear": 0.01, "relative_to_bounding_box": true, "average_normals": true},
        "skew": {"method": "MeshNormalsBased", "edge_protection": false, "edge_distance": 0.15, "edge_contrast": 0.5},
        "id": {"source": "MaterialColor", "generator": "HueShift"},
        "ao": {"rays": rays, "ignore_backface": "Never", "self_occlusion": "Always", "attenuation": "Linear", "ground_plane": {"enabled": false, "offset": 0.0}},
        "curvature": {
            "method": "NormalMap",
            "normal_map": {"details": 0.5, "enable_seams": true, "seams_intensity": 1.0},
            "mesh": {"rays": 32, "sampling_radius": 0.001, "relative_to_bounding_box": true, "self_intersection": "Always", "auto_tonemapping": true, "value_min": -1.0, "value_max": 1.0}
        },
        "position": {"axis": "All", "normalization": "BoundingSphere", "scale": "FullScene"},
        "thickness": {"rays": thickness_rays, "self_occlusion": "Always", "normalization": "RelativeToMinMax"},
        "height": {"normalization": "RelativeToLowMesh", "scaling_divisor": 1.0},
        "bent_normal": {"rays": rays, "ignore_backface": "Never", "self_occlusion": "Always"}
    });

    let mut maps = serde_json::Map::new();
    for key in ["id", "ao", "curvature", "position", "thickness", "height", "bent_normal"] {
        let parameters = expected.as_object_mut().unwrap().remove(key).unwrap();
        maps.insert(key.into(), json!({"is_enabled": !["height", "bent_normal"].contains(&key), "parameters": parameters}));
    }
    for key in ["normal", "world_normal", "opacity"] {
        maps.insert(key.into(), json!({"is_enabled": key != "opacity", "parameters": null}));
    }
    expected["maps"] = Value::Object(maps);

    // Normalize the expected JSON floats to f32 before comparing every value
    let decoded: bake::BakeSettings = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(bake::BakeSettings::default(), decoded);
    assert_eq!(serde_json::from_str::<bake::BakeSettings>("{}").unwrap(), decoded);
    assert_eq!(decoded.common.output_size(), (2048, 2048));

    fn assert_shape(actual: &Value, expected: &Value) {
        if let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) {
            assert_eq!(actual.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>());
            for (key, expected) in expected {
                assert_shape(&actual[key], expected);
            }
        }
    }
    assert_shape(&serde_json::to_value(decoded).unwrap(), &expected);
    assert_eq!(BakedMap::ALL.len(), 10);
    for name in ["color_map", "Color map", "WorldDirection", "world_space_direction"] {
        assert_eq!(BakedMap::parse(name), None);
    }
}

fn assert_choices<T: Copy + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned>(choices: &[T]) {
    for choice in choices {
        let encoded = serde_json::to_value(choice).unwrap();
        assert!(encoded.is_string());
        assert_eq!(serde_json::from_value::<T>(encoded).unwrap(), *choice);
    }

    for invalid in [json!(true), json!(false), json!(1), json!(3), json!("invalid")] {
        assert!(serde_json::from_value::<T>(invalid).is_err());
    }
}

#[test]
fn all_dropdowns_store_enum_choices() {
    assert_choices(&[bake::MeshMatchMode::Always, bake::MeshMatchMode::ByMeshName]);
    assert_choices(&[bake::SkewCorrectionMethod::MeshNormalsBased, bake::SkewCorrectionMethod::FaceNormalsBased]);
    assert_choices(&bake::BakeResolution::ALL);
    assert_choices(&bake::BakeAntialiasing::ALL);
    assert_choices(&[bake::CageType::DistanceBased, bake::CageType::CustomFile]);
    assert_choices(&[bake::CurvatureMethod::NormalMap, bake::CurvatureMethod::Mesh]);
    assert_choices(&[
        bake::IdColorSource::VertexColor,
        bake::IdColorSource::MaterialColor,
        bake::IdColorSource::FileId,
        bake::IdColorSource::MeshId,
        bake::IdColorSource::Submesh,
    ]);
    assert_choices(&[bake::IdColorGenerator::Random, bake::IdColorGenerator::HueShift, bake::IdColorGenerator::Grayscale]);
    assert_choices(&[bake::RayDistribution::Uniform, bake::RayDistribution::Cosine]);
    assert_choices(&[bake::BackfaceMode::Never, bake::BackfaceMode::Always, bake::BackfaceMode::ByMeshName]);
    assert_choices(&[bake::SelfOcclusion::Always, bake::SelfOcclusion::SameMeshName]);
    assert_choices(&[bake::AmbientOcclusionAttenuation::None, bake::AmbientOcclusionAttenuation::Smooth, bake::AmbientOcclusionAttenuation::Linear]);
    assert_choices(&[bake::PositionAxis::All, bake::PositionAxis::X, bake::PositionAxis::Y, bake::PositionAxis::Z]);
    assert_choices(&[bake::PositionMode::BoundingBox, bake::PositionMode::BoundingSphere, bake::PositionMode::None]);
    assert_choices(&[bake::PositionNormalizationScale::PerMaterial, bake::PositionNormalizationScale::FullScene]);
    assert_choices(&[bake::ThicknessNormalization::RelativeToRayDistances, bake::ThicknessNormalization::RelativeToMinMax, bake::ThicknessNormalization::None]);
    assert_choices(&[
        bake::HeightNormalization::RelativeToRayDistances,
        bake::HeightNormalization::RelativeToLowMesh,
        bake::HeightNormalization::RelativeToMinMax,
        bake::HeightNormalization::Manual,
    ]);
}

#[test]
fn components_keep_their_own_defaults() {
    let settings: bake::BakeSettings = serde_json::from_value(json!({
        "common": {}, "cage": {}, "high_poly": {}, "skew": {},
        "maps": {
            "ao": {"is_enabled": true, "parameters": {"ground_plane": {}}},
            "curvature": {"is_enabled": true, "parameters": {"normal_map": {}, "mesh": {}}}
        }
    }))
    .unwrap();
    assert_eq!(settings, bake::BakeSettings::default());
    assert_eq!(settings.maps.thickness.parameters.rays.max_distance, 0.1);
}

#[test]
fn nested_settings_round_trip_per_texture_set() {
    let mut settings = bake::BakeSettings::default();
    settings.common.resolution = bake::BakeResolution::R16;
    settings.common.resolution_height = Some(bake::BakeResolution::R8192);
    settings.common.antialiasing = bake::BakeAntialiasing::Subsampling8x8;
    settings.common.apply_diffusion = false;
    settings.high_poly.ignore_backface_suffix = "_two_sided".into();
    settings.cage.kind = bake::CageType::CustomFile;
    settings.skew.edge_protection = true;
    settings.maps.id.parameters.source = bake::IdColorSource::Submesh;
    settings.maps.id.parameters.generator = bake::IdColorGenerator::Grayscale;
    settings.maps.ao.parameters.ignore_backface = bake::BackfaceMode::ByMeshName;
    settings.maps.ao.parameters.ground_plane = bake::GroundPlaneSettings { enabled: true, offset: 5.0 };
    settings.maps.curvature.parameters.method = bake::CurvatureMethod::Mesh;
    settings.maps.curvature.parameters.normal_map.details = 0.25;
    settings.maps.curvature.parameters.mesh.sampling_radius = 0.25;
    settings.maps.curvature.parameters.mesh.auto_tonemapping = false;
    settings.maps.position.parameters.axis = bake::PositionAxis::Z;
    settings.maps.position.parameters.normalization = bake::PositionMode::None;
    settings.maps.thickness.parameters.normalization = bake::ThicknessNormalization::None;
    settings.maps.height.parameters.normalization = bake::HeightNormalization::Manual;
    settings.maps.height.parameters.scaling_divisor = 2.0;
    settings.maps.bent_normal.parameters.ignore_backface = bake::BackfaceMode::ByMeshName;

    let mut project = Project::new("Bake options", "builtin:plane");
    project.texture_sets.push(TextureSet::new("A", 0, 128));
    project.texture_sets.push(TextureSet::new("B", 1, 128));
    project.texture_sets[0].bake = settings;

    let reloaded: Project = serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
    assert_eq!(project, reloaded);
    assert_eq!(reloaded.texture_sets[0].bake.common.output_size(), (16, 8192));
    assert_eq!(reloaded.texture_sets[1].bake, bake::BakeSettings::default());
}

#[test]
fn malformed_settings_are_rejected() {
    for invalid in [
        json!({"maps": {"ao": {"is_enabled": "yes", "parameters": {}}}}),
        json!({"maps": {"ao": {"is_enabled": true, "parameters": {"rays": {"min_distance": "bad"}}}}}),
        json!({"common": {"resolution_height": -1}}),
        json!({"maps": {"curvature": {"is_enabled": true, "parameters": {"method": true}}}}),
        Value::Null,
    ] {
        assert!(serde_json::from_value::<bake::BakeSettings>(invalid).is_err());
    }
}

#[test]
fn map_enablement_is_owned_by_each_map() {
    let mut settings = bake::BakeSettings::default();
    settings.maps.ao.parameters.rays.rays = 23;
    settings.maps.ao.is_enabled = false;
    settings.maps.height.is_enabled = true;
    assert!(!settings.maps.enabled().any(|map| map == BakedMap::AmbientOcclusion));
    assert!(settings.maps.enabled().any(|map| map == BakedMap::Height));

    let encoded = serde_json::to_value(&settings).unwrap();
    assert_eq!(encoded["maps"]["ao"]["is_enabled"], false);
    assert!(encoded.get("selection").is_none());
    let mut reloaded: bake::BakeSettings = serde_json::from_value(encoded).unwrap();
    assert_eq!(reloaded, settings);

    reloaded.maps.select([BakedMap::AmbientOcclusion]);
    assert_eq!(reloaded.maps.enabled().collect::<Vec<_>>(), [BakedMap::AmbientOcclusion]);
    assert!(reloaded.maps.ao.is_enabled);
    assert_eq!(reloaded.maps.ao.parameters.rays.rays, 23);
}
