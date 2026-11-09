use super::controls::{choice, hint, integer, number, section, text_input};
use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::widgets::icon_button;
use egui::Ui;
use egui_phosphor::regular as icon;
use serde_json::{Map, Value, json};
use texturecraft_engine::core::{
    BakeAntialiasing, BakePaintTarget, BakeResolution, BakeSettings, CageType, CommonBakeSettings, MeshMatchMode, SkewCorrectionMethod,
};

/// The output and high-poly sections, then cage (needs a high poly) and skew (needs a distance cage)
pub(super) fn show(app: &mut TextureCraftApp, ui: &mut Ui, settings: &mut BakeSettings) -> Option<Value> {
    let mut reset = Map::new();
    ui.push_id("common", |ui| {
        // Restore Defaults keeps whether the texture set is included in the bake
        let is_enabled = settings.common.is_enabled;
        if section(ui, tr("Common Parameters"), |ui| {
            output_size(ui, &mut settings.common);
            choice(
                ui,
                tr("Antialiasing"),
                &mut settings.common.antialiasing,
                &[
                    (BakeAntialiasing::None, tr("None")),
                    (BakeAntialiasing::Subsampling2x2, tr("Subsampling 2x2")),
                    (BakeAntialiasing::Subsampling4x4, tr("Subsampling 4x4")),
                    (BakeAntialiasing::Subsampling8x8, tr("Subsampling 8x8")),
                ],
            );
            integer(ui, tr("Dilation Width"), &mut settings.common.dilation, 0..=128);
            ui.checkbox(&mut settings.common.apply_diffusion, tr("Apply Diffusion"));
        }) {
            settings.common = CommonBakeSettings { is_enabled, ..CommonBakeSettings::default() };
            let mut defaults = json!(settings.common);
            if let Some(fields) = defaults.as_object_mut() {
                fields.remove("is_enabled");
            }
            reset.insert("common".into(), defaults);
        }
    });

    ui.add_space(8.0);
    ui.push_id("high_poly", |ui| {
        if section(ui, tr("High-Poly Parameters"), |ui| {
            ui.checkbox(&mut settings.high_poly.use_low_as_high, tr("Use Low Poly Mesh as High Poly Mesh"));
            high_poly_files(app, ui);
            if !app.session.bake_meshes.high.is_empty() {
                ui.checkbox(&mut settings.high_poly.ignore_backface, tr("Ignore Backface"));
                choice(
                    ui,
                    tr("Match"),
                    &mut settings.high_poly.match_mode,
                    &[(MeshMatchMode::Always, tr("Always")), (MeshMatchMode::ByMeshName, tr("By Mesh Name"))],
                );
                text_input(ui, tr("Low Poly Mesh Suffix"), &mut settings.high_poly.low_suffix);
                text_input(ui, tr("High Poly Mesh Suffix"), &mut settings.high_poly.high_suffix);
                text_input(ui, tr("Ignore Backface Suffix"), &mut settings.high_poly.ignore_backface_suffix);
            }
        }) {
            settings.high_poly = Default::default();
            reset.insert("high_poly".into(), json!(settings.high_poly));
        }
    });

    if !app.session.bake_meshes.high.is_empty() {
        ui.add_space(8.0);
        ui.push_id("cage", |ui| {
            if section(ui, tr("Cage"), |ui| {
                choice(
                    ui,
                    tr("Cage Type"),
                    &mut settings.cage.kind,
                    &[(CageType::DistanceBased, tr("Distance-Based")), (CageType::CustomFile, tr("Custom File"))],
                );
                match settings.cage.kind {
                    CageType::DistanceBased => {
                        paint_brush(app, ui, BakePaintTarget::Cage);
                        number(ui, tr("Max Frontal Distance"), &mut settings.cage.max_frontal, 0.0..=0.05, 4);
                        number(ui, tr("Max Rear Distance"), &mut settings.cage.max_rear, 0.0..=0.05, 4);
                        ui.checkbox(&mut settings.cage.relative_to_bounding_box, tr("Relative to Bounding Box"));
                        ui.checkbox(&mut settings.cage.average_normals, tr("Average Normals"));
                    }
                    CageType::CustomFile => cage_file(app, ui),
                }
            }) {
                settings.cage = Default::default();
                reset.insert("cage".into(), json!(settings.cage));
            }
        });

        if settings.cage.kind == CageType::DistanceBased {
            ui.add_space(8.0);
            ui.push_id("skew", |ui| {
                if section(ui, tr("Skew Correction"), |ui| {
                    choice(
                        ui,
                        tr("Skew Correction Method"),
                        &mut settings.skew.method,
                        &[
                            (SkewCorrectionMethod::MeshNormalsBased, tr("Mesh Normals Based")),
                            (SkewCorrectionMethod::FaceNormalsBased, tr("Face Normals Based")),
                        ],
                    );
                    paint_brush(app, ui, BakePaintTarget::Skew);
                    ui.checkbox(&mut settings.skew.edge_protection, tr("Edge Protection"));
                    number(ui, tr("Edge Distance"), &mut settings.skew.edge_distance, 0.0..=1.0, 2);
                    number(ui, tr("Edge Contrast"), &mut settings.skew.edge_contrast, 0.0..=1.0, 2);
                }) {
                    settings.skew = Default::default();
                    reset.insert("skew".into(), json!(settings.skew));
                }
            });
        }
    }

    (!reset.is_empty()).then_some(Value::Object(reset))
}

fn output_size(ui: &mut Ui, settings: &mut CommonBakeSettings) {
    ui.label(tr("Output Size"));
    ui.horizontal(|ui| {
        let button_size = ui.spacing().interact_size.y;
        let dropdown_width = ((ui.available_width() - button_size - ui.spacing().item_spacing.x * 2.0) / 2.0).max(40.0);
        let resolution = |ui: &mut Ui, id: &str, value: &mut BakeResolution| {
            egui::ComboBox::from_id_salt(id).width(dropdown_width).selected_text(value.pixels().to_string()).show_ui(ui, |ui| {
                for resolution in BakeResolution::ALL {
                    ui.selectable_value(value, resolution, resolution.pixels().to_string());
                }
            });
        };

        resolution(ui, "output_width", &mut settings.resolution);
        let locked = settings.resolution_height.is_none();
        let mut height = settings.resolution_height.unwrap_or(settings.resolution);
        ui.add_enabled_ui(!locked, |ui| resolution(ui, "output_height", &mut height));
        if !locked {
            settings.resolution_height = Some(height);
        }

        if icon_button(ui, if locked { icon::LOCK_SIMPLE } else { icon::LOCK_SIMPLE_OPEN }, tr("Lock Output Size"), locked, button_size).clicked() {
            settings.resolution_height = if locked { Some(settings.resolution) } else { None };
        }
    });
}

fn high_poly_files(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.label(tr("High Poly Scene"));
    let meshes: Vec<_> = app.session.bake_meshes.high.iter().map(|mesh| mesh.name.clone()).collect();
    let mut remove = None;
    for (index, name) in meshes.iter().enumerate() {
        ui.horizontal(|ui| {
            let button_size = ui.spacing().interact_size.y;
            let label_width = (ui.available_width() - button_size - ui.spacing().item_spacing.x).max(0.0);
            ui.add_sized([label_width, ui.spacing().interact_size.y], egui::Label::new(name).truncate()).on_hover_text(name);
            if icon_button(ui, icon::TRASH, tr("Remove"), false, button_size).clicked() {
                remove = Some(index);
            }
        });
    }

    if meshes.is_empty() {
        hint(ui, tr("No High-Poly Meshes"));
    }
    if let Some(index) = remove {
        app.run("bake.highPoly.remove", json!({ "index": index }));
    }
    if ui.button(tr("Add High-Poly Meshes")).clicked() {
        crate::web_files::import_bake_mesh(app, &ui.ctx().clone(), false);
    }
}

fn cage_file(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.label(tr("Cage Scene"));
    let cage = app.session.bake_meshes.cage.as_ref().map(|mesh| mesh.name.clone());
    ui.horizontal(|ui| {
        let button_size = ui.spacing().interact_size.y;
        let label_width = (ui.available_width() - 2.0 * (button_size + ui.spacing().item_spacing.x)).max(0.0);
        ui.add_sized([label_width, ui.spacing().interact_size.y], egui::Label::new(cage.as_deref().unwrap_or("")).truncate());
        if icon_button(ui, icon::FOLDER_OPEN, tr("Load Cage"), false, button_size).clicked() {
            crate::web_files::import_bake_mesh(app, &ui.ctx().clone(), true);
        }
        if cage.is_some() && icon_button(ui, icon::TRASH, tr("Remove Cage"), false, button_size).clicked() {
            app.run("bake.cage.clear", json!({}));
        }
    });
}

fn paint_brush(app: &mut TextureCraftApp, ui: &mut Ui, target: BakePaintTarget) {
    let (label, clear_label, clear_command) = match target {
        BakePaintTarget::Cage => ("Paint Cage", "Reset Painted Cage", "bake.cage.reset"),
        BakePaintTarget::Skew => ("Paint Skew Correction", "Remove Skew Correction", "bake.skew.clear"),
    };
    let mut brush = app.ui.bake_paint_brush;
    let active = brush.on && brush.target == target;
    if ui.add(egui::Button::new(tr(label)).selected(active)).clicked() {
        brush.on = !active;
        if brush.target != target {
            brush.value = match target {
                BakePaintTarget::Cage => 0.5,
                BakePaintTarget::Skew => 1.0,
            };
        }
        brush.target = target;
    }
    if brush.on && brush.target == target {
        number(ui, tr("Radius"), &mut brush.radius, 0.002..=0.2, 3);
        number(ui, tr("Value"), &mut brush.value, 0.0..=1.0, 2);
        number(ui, tr("Hardness"), &mut brush.hardness, 0.0..=1.0, 2);
        if app.ui.layout == crate::state::ViewLayout::Only3D {
            app.ui.layout = crate::state::ViewLayout::Split;
        }
    }
    app.ui.bake_paint_brush = brush;
    if app.session.doc.bake_paint.contains_key(&(app.session.ts(), target)) && ui.button(tr(clear_label)).clicked() {
        app.run(clear_command, json!({}));
    }
}
