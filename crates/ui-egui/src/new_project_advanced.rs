//! New project ▸ Advanced settings: the normal map format, and baked map files imported at creation

use std::collections::BTreeSet;

use egui::{RichText, Sense, Ui, vec2};
use serde_json::{Value, json};
use texturecraft_engine::core::NormalFormat;

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::state::NewProject;
use crate::theme::{Tokens, caption};
use crate::widgets::{section, section_chevron};
use crate::windows::file_dialogs_available;

/// Labels of every New project form row, so Basic and Advanced share one label column
const FORM_LABELS: [&str; 6] = ["3D mesh", "Project template", "Document resolution", "UV tiles (UDIM)", "Name", "Normal map format"];

/// Each normal map format with its command id and label
const NORMAL_FORMATS: [(NormalFormat, &str, &str); 2] = [(NormalFormat::OpenGl, "openGl", "OpenGL"), (NormalFormat::DirectX, "directX", "DirectX")];

/// Width of the dialog's dropdowns
pub(crate) const COMBO_WIDTH: f32 = 200.0;

/// Width of the Add... and Clear buttons
const FILE_BUTTON_WIDTH: f32 = 96.0;

/// Tallest the list of added files grows before it scrolls
const FILE_LIST_HEIGHT: f32 = 160.0;

/// Width of the widest form label, for the label column of every form grid in the dialog
pub(crate) fn form_label_width(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    FORM_LABELS.iter().map(|label| ui.painter().layout_no_wrap(tr(label).to_string(), font.clone(), egui::Color32::WHITE).size().x).fold(0.0, f32::max)
}

/// The collapsible Advanced settings section (closed until opened)
pub(crate) fn advanced_settings(app: &mut TextureCraftApp, ui: &mut Ui) {
    let id = ui.make_persistent_id("np_advanced");
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let header = ui
        .horizontal(|ui| {
            ui.add(egui::Label::new(section_chevron(ui, state.is_open())).selectable(false));
            ui.add(egui::Label::new(caption(tr("Advanced settings"))).selectable(false));
        })
        .response
        .interact(Sense::click());
    if header.clicked() {
        state.toggle(ui);
    }

    state.show_body_unindented(ui, |ui| {
        egui::Grid::new("np_advanced_grid").num_columns(2).spacing(vec2(12.0, 8.0)).min_col_width(form_label_width(ui)).show(ui, |ui| {
            ui.label(tr("Normal map format"));
            normal_format_combo(app, ui);
            ui.end_row();
        });
        if file_dialogs_available() {
            ui.add_space(4.0);
            section(ui, "np_baked_maps", tr("Import baked maps"), false, |ui| {
                if baked_maps(&mut app.ui.new_project, ui) {
                    ask_baked_map_files(app);
                }
            });
        }
    });
}

fn normal_format_combo(app: &mut TextureCraftApp, ui: &mut Ui) {
    let new_project = &mut app.ui.new_project;
    let template_format = app.session.template(&new_project.template).map(|template| template.normal_format).unwrap_or_default();
    let current = new_project.normal_format.unwrap_or(template_format);
    let current_label = NORMAL_FORMATS.iter().find(|(format, _, _)| *format == current).map_or("", |(_, _, label)| label);
    egui::ComboBox::from_id_salt("np_normal_format").width(COMBO_WIDTH).selected_text(current_label).show_ui(ui, |ui| {
        for (format, _, label) in NORMAL_FORMATS {
            if ui.selectable_label(format == current, label).clicked() {
                new_project.normal_format = Some(format);
            }
        }
    });
}

/// The baked map list and its buttons; true when Add was clicked.
fn baked_maps(new_project: &mut NewProject, ui: &mut Ui) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let mut add = false;
    ui.horizontal(|ui| {
        let buttons_width = 2.0 * FILE_BUTTON_WIDTH + ui.spacing().item_spacing.x;
        let hint_width = (ui.available_width() - buttons_width - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(vec2(hint_width, 0.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(hint_width);
            ui.add(egui::Label::new(RichText::new(tr("Name each file after its texture set, like Body_normal.png")).color(tokens.text_dim)).wrap());
        });
        if ui.add(egui::Button::new(tr("Add...")).min_size(vec2(FILE_BUTTON_WIDTH, 0.0))).clicked() {
            add = true;
        }
        if ui.add_enabled(!new_project.baked_maps.is_empty(), egui::Button::new(tr("Clear")).min_size(vec2(FILE_BUTTON_WIDTH, 0.0))).clicked() {
            new_project.baked_maps.clear();
        }
    });
    if new_project.baked_maps.is_empty() {
        return add;
    }

    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(FILE_LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
        for path in &new_project.baked_maps {
            ui.add(egui::Label::new(file_name(path)).truncate()).on_hover_text(path.as_str());
        }
    });
    add
}

/// List picked baked map files (files already listed are skipped)
pub fn add_baked_map_files(new_project: &mut NewProject, paths: Vec<String>) {
    for path in paths {
        if !new_project.baked_maps.contains(&path) {
            new_project.baked_maps.push(path);
        }
    }
}

/// After creation: set the normal format, import the listed baked maps (one undo step), toast the outcome; a failed import keeps the list
pub(crate) fn apply_after_create(app: &mut TextureCraftApp, new_project: &NewProject) {
    let chosen_format = new_project.normal_format.filter(|format| *format != app.session.doc.project.normal_format);
    if let Some((_, id, _)) = chosen_format.and_then(|chosen| NORMAL_FORMATS.iter().find(|(format, _, _)| *format == chosen)) {
        app.run("textureSet.settings", json!({ "normalFormat": id }));
    }
    if new_project.baked_maps.is_empty() {
        return;
    }

    let files: Vec<Value> = new_project.baked_maps.iter().map(|path| json!({ "path": path })).collect();
    let Some(result) = app.run("textureSet.meshMap.import", json!({ "files": files })) else {
        return;
    };

    let assigned = result["assigned"].as_array().into_iter().flatten();
    let maps: BTreeSet<(u64, &str)> = assigned.filter_map(|entry| Some((entry["ts"].as_u64()?, entry["map"].as_str()?))).collect();
    let not_imported: Vec<String> = result["unmatched"].as_array().into_iter().flatten().filter_map(Value::as_str).map(file_name).collect();
    let mut text = trf("Baked maps imported: {count}", &[("count", &maps.len())]);
    if !not_imported.is_empty() {
        text.push('\n');
        text.push_str(&trf("Not imported: {files}", &[("files", &not_imported.join(", "))]));
    }
    app.toast(text, !not_imported.is_empty());
    app.ui.new_project.baked_maps.clear();
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn ask_baked_map_files(app: &mut TextureCraftApp) {
    use crate::file_dialogs::{Bound, FileDialog};
    app.ask_file(FileDialog::open_many().filter(tr("Images"), crate::web_files::IMAGE_EXTS), Bound::App, |app, files| {
        add_baked_map_files(&mut app.ui.new_project, files.into_iter().map(|path| path.display().to_string()).collect());
    });
}

#[cfg(target_arch = "wasm32")]
fn ask_baked_map_files(_app: &mut TextureCraftApp) {}
