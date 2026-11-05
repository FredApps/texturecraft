use super::controls::{choice, integer, number, section, section_header};
use crate::i18n::{baked_map_label, tr};
use egui::Ui;
use texturecraft_engine::core::{
    AmbientOcclusionAttenuation, AmbientOcclusionSettings, BackfaceMode, BakedMap, BentNormalSettings, CurvatureMethod, CurvatureSettings, HeightNormalization,
    HeightSettings, IdColorGenerator, IdColorSource, IdSettings, MapSettings, MeshMapSettings, PositionAxis, PositionMode, PositionNormalizationScale,
    PositionSettings, RayDistribution, RaySettings, SelfOcclusion, ThicknessNormalization, ThicknessSettings,
};

pub(super) fn show(ui: &mut Ui, map: BakedMap, settings: &mut MeshMapSettings) {
    match map {
        BakedMap::Normal | BakedMap::WorldNormal | BakedMap::Opacity => {
            let mut is_enabled = settings.is_enabled(map);
            section_header(ui, tr(baked_map_label(map)), Some(&mut is_enabled), None);
            settings.set_enabled(map, is_enabled);
        }
        BakedMap::Id => baker(ui, map, &mut settings.id, id),
        BakedMap::AmbientOcclusion => baker(ui, map, &mut settings.ao, ambient_occlusion),
        BakedMap::Curvature => baker(ui, map, &mut settings.curvature, curvature),
        BakedMap::Position => baker(ui, map, &mut settings.position, position),
        BakedMap::Thickness => baker(ui, map, &mut settings.thickness, thickness),
        BakedMap::Height => baker(ui, map, &mut settings.height, height),
        BakedMap::BentNormal => baker(ui, map, &mut settings.bent_normal, bent_normal),
    }
}

fn baker<T: Default>(ui: &mut Ui, map: BakedMap, settings: &mut MapSettings<T>, controls: impl FnOnce(&mut Ui, &mut T)) {
    if section(ui, tr(baked_map_label(map)), Some(&mut settings.is_enabled), |ui| controls(ui, &mut settings.parameters)) {
        settings.parameters = T::default();
    }
}

fn id(ui: &mut Ui, settings: &mut IdSettings) {
    choice(
        ui,
        tr("Color Source"),
        &mut settings.source,
        &[
            (IdColorSource::VertexColor, tr("Vertex Color")),
            (IdColorSource::MaterialColor, tr("Material Color")),
            (IdColorSource::FileId, tr("File ID")),
            (IdColorSource::MeshId, tr("Mesh ID")),
            (IdColorSource::Submesh, tr("Submesh")),
        ],
    );
    if matches!(settings.source, IdColorSource::FileId | IdColorSource::MeshId | IdColorSource::Submesh) {
        choice(
            ui,
            tr("Color Generator"),
            &mut settings.generator,
            &[(IdColorGenerator::Random, tr("Random")), (IdColorGenerator::HueShift, tr("Hue Shift")), (IdColorGenerator::Grayscale, tr("Grayscale"))],
        );
    }
}

fn rays(ui: &mut Ui, settings: &mut RaySettings) {
    integer(ui, tr("Secondary Rays"), &mut settings.rays, 1..=256);
    number(ui, tr("Min Occluder Distance"), &mut settings.min_distance, 0.00001..=1.0, 5);
    number(ui, tr("Max Occluder Distance"), &mut settings.max_distance, 0.00001..=if settings.relative_to_bounding_box { 1.0 } else { 10.0 }, 5);
    if ui.checkbox(&mut settings.relative_to_bounding_box, tr("Relative to Bounding Box")).changed() && settings.relative_to_bounding_box {
        settings.max_distance = settings.max_distance.min(1.0);
    }
    number(ui, tr("Spread Angle"), &mut settings.spread, 0.0..=180.0, 0);
    choice(ui, tr("Distribution"), &mut settings.distribution, &[(RayDistribution::Uniform, tr("Uniform")), (RayDistribution::Cosine, tr("Cosine"))]);
}

fn backface(ui: &mut Ui, settings: &mut BackfaceMode) {
    choice(
        ui,
        tr("Ignore Backface"),
        settings,
        &[(BackfaceMode::Never, tr("Never")), (BackfaceMode::Always, tr("Always")), (BackfaceMode::ByMeshName, tr("By Mesh Name"))],
    );
}

fn occlusion(ui: &mut Ui, label: &str, settings: &mut SelfOcclusion) {
    choice(ui, label, settings, &[(SelfOcclusion::Always, tr("Always")), (SelfOcclusion::SameMeshName, tr("Only Same Mesh Name"))]);
}

fn ambient_occlusion(ui: &mut Ui, settings: &mut AmbientOcclusionSettings) {
    rays(ui, &mut settings.rays);
    backface(ui, &mut settings.ignore_backface);
    occlusion(ui, tr("Self Occlusion"), &mut settings.self_occlusion);
    choice(
        ui,
        tr("Attenuation"),
        &mut settings.attenuation,
        &[
            (AmbientOcclusionAttenuation::None, tr("None")),
            (AmbientOcclusionAttenuation::Smooth, tr("Smooth")),
            (AmbientOcclusionAttenuation::Linear, tr("Linear")),
        ],
    );
    ui.checkbox(&mut settings.ground_plane.enabled, tr("Ground Plane"));
    if settings.ground_plane.enabled {
        number(ui, tr("Ground Plane Offset"), &mut settings.ground_plane.offset, 0.0..=100.0, 2);
    }
}

fn curvature(ui: &mut Ui, settings: &mut CurvatureSettings) {
    choice(
        ui,
        tr("Method"),
        &mut settings.method,
        &[(CurvatureMethod::NormalMap, tr("Generate from Normal Map")), (CurvatureMethod::Mesh, tr("Generate from Mesh"))],
    );
    match settings.method {
        CurvatureMethod::NormalMap => {
            number(ui, tr("Details"), &mut settings.normal_map.details, 0.0..=1.0, 2);
            ui.checkbox(&mut settings.normal_map.enable_seams, tr("Enable Seams"));
            if settings.normal_map.enable_seams {
                number(ui, tr("Seams Intensity"), &mut settings.normal_map.seams_intensity, 0.0..=1.0, 2);
            }
        }
        CurvatureMethod::Mesh => {
            let mesh = &mut settings.mesh;
            integer(ui, tr("Secondary Rays"), &mut mesh.rays, 1..=256);
            number(ui, tr("Sampling Radius"), &mut mesh.sampling_radius, 0.00001..=if mesh.relative_to_bounding_box { 10.0 } else { 1.0 }, 5);
            if ui.checkbox(&mut mesh.relative_to_bounding_box, tr("Relative to Bounding Box")).changed() && !mesh.relative_to_bounding_box {
                mesh.sampling_radius = mesh.sampling_radius.min(1.0);
            }
            occlusion(ui, tr("Self Intersection"), &mut mesh.self_intersection);
            ui.checkbox(&mut mesh.auto_tonemapping, tr("Auto Tonemapping"));
            if !mesh.auto_tonemapping {
                number(ui, tr("Value Bounds Min"), &mut mesh.value_min, -10.0..=10.0, 2);
                number(ui, tr("Value Bounds Max"), &mut mesh.value_max, -10.0..=10.0, 2);
            }
        }
    }
}

fn position(ui: &mut Ui, settings: &mut PositionSettings) {
    choice(ui, tr("Axis"), &mut settings.axis, &[(PositionAxis::All, tr("All")), (PositionAxis::X, "X"), (PositionAxis::Y, "Y"), (PositionAxis::Z, "Z")]);
    choice(
        ui,
        tr("Normalization Type"),
        &mut settings.normalization,
        &[(PositionMode::BoundingBox, tr("BBox")), (PositionMode::BoundingSphere, tr("BSphere")), (PositionMode::None, tr("None"))],
    );
    choice(
        ui,
        tr("Normalization Scale"),
        &mut settings.scale,
        &[(PositionNormalizationScale::PerMaterial, tr("Per Material")), (PositionNormalizationScale::FullScene, tr("Full Scene"))],
    );
}

fn thickness(ui: &mut Ui, settings: &mut ThicknessSettings) {
    rays(ui, &mut settings.rays);
    occlusion(ui, tr("Self Occlusion"), &mut settings.self_occlusion);
    choice(
        ui,
        tr("Normalization"),
        &mut settings.normalization,
        &[
            (ThicknessNormalization::RelativeToRayDistances, tr("Relative to Ray Distances")),
            (ThicknessNormalization::RelativeToMinMax, tr("Relative to Min/Max")),
            (ThicknessNormalization::None, tr("None")),
        ],
    );
}

fn height(ui: &mut Ui, settings: &mut HeightSettings) {
    choice(
        ui,
        tr("Normalization"),
        &mut settings.normalization,
        &[
            (HeightNormalization::RelativeToRayDistances, tr("Relative to Ray Distances")),
            (HeightNormalization::RelativeToLowMesh, tr("Relative to Low Mesh")),
            (HeightNormalization::RelativeToMinMax, tr("Relative to Min/Max")),
            (HeightNormalization::Manual, tr("Manual")),
        ],
    );
    if settings.normalization == HeightNormalization::Manual {
        number(ui, tr("Scaling Divisor"), &mut settings.scaling_divisor, 0.00001..=100.0, 5);
    }
}

fn bent_normal(ui: &mut Ui, settings: &mut BentNormalSettings) {
    rays(ui, &mut settings.rays);
    backface(ui, &mut settings.ignore_backface);
    occlusion(ui, tr("Self Occlusion"), &mut settings.self_occlusion);
}
