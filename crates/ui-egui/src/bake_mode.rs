//! Bake Mode presentation; persisted settings live in core and edits use engine commands

mod bake_paint_brush;
mod common_settings;
pub use bake_paint_brush::BakePaintBrush;
mod controls;
mod map_settings;

use egui::{Ui, vec2};
use serde_json::json;
use texturecraft_engine::core::{BakeSettings, BakedMap};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::state::Mode;
use crate::widgets::primary_button;

pub fn texture_sets(app: &mut TextureCraftApp, ui: &mut Ui) {
    egui::ScrollArea::vertical().id_salt("bake_texture_sets").auto_shrink([false; 2]).show(ui, |ui| {
        for set_index in 0..app.session.set_count() {
            let Some(texture_set) = app.session.doc.project.texture_sets.get(set_index) else { continue };

            let name = texture_set.name.clone();
            let (width, height) = texture_set.bake.common.output_size();
            let mut is_enabled = texture_set.bake.common.is_enabled;
            ui.push_id(set_index, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut is_enabled, "").on_hover_text(tr("Include in Bake")).changed() {
                        app.run("bake.settings", json!({ "ts": set_index, "enabled": is_enabled }));
                    }
                    if ui.selectable_label(set_index == app.session.ts(), format!("{name}  ({width} × {height})")).clicked() {
                        app.run("textureSet.select", json!({ "ts": set_index }));
                    }
                });
            });
        }
    });
}

pub fn bake_body(app: &mut TextureCraftApp, ui: &mut Ui) {
    edit_settings(app, ui, "bake_maps", |_, ui, settings| {
        for map in BakedMap::ALL {
            ui.push_id(map, |ui| map_settings::show(ui, map, &mut settings.maps));
            ui.add_space(6.0);
        }
    });
}

pub fn settings_body(app: &mut TextureCraftApp, ui: &mut Ui) {
    edit_settings(app, ui, "bake_common_settings", common_settings::show);
}

fn edit_settings(app: &mut TextureCraftApp, ui: &mut Ui, scroll_id: &str, contents: impl FnOnce(&mut TextureCraftApp, &mut Ui, &mut BakeSettings)) {
    let set_index = app.session.ts();
    let Some(before) = app.session.doc.project.texture_sets.get(set_index).map(|texture_set| texture_set.bake.clone()) else { return };

    let mut settings = before.clone();
    egui::ScrollArea::vertical().id_salt(scroll_id).auto_shrink([false; 2]).show(ui, |ui| contents(app, ui, &mut settings));
    if settings != before {
        app.run("bake.settings", json!({ "ts": set_index, "model": settings }));
    }
}

pub(crate) fn bake_bar(app: &mut TextureCraftApp, ctx: &egui::Context, viewport: egui::Rect) {
    egui::Area::new(egui::Id::unique("bake_bar")).constrain_to(viewport).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -16.0)).default_width(0.0).show(
        ctx,
        |ui| {
            ui.horizontal(|ui| {
                if let Some(progress) = app.bake_progress() {
                    ui.add(egui::ProgressBar::new(progress).desired_width(220.0).show_percentage());
                    if ui.button(tr("Cancel")).clicked() {
                        app.cancel_bake();
                    }
                } else {
                    let texture_sets: Vec<usize> = app
                        .session
                        .doc
                        .project
                        .texture_sets
                        .iter()
                        .enumerate()
                        .filter(|(_, texture_set)| texture_set.bake.common.is_enabled)
                        .map(|(index, _)| index)
                        .collect();
                    let label = trf("Bake Selected Textures ({n})", &[("n", &texture_sets.len())]);
                    if ui.add_enabled(!texture_sets.is_empty(), primary_button(ui, &label)).clicked() {
                        app.start_bake(texture_sets);
                    }
                }

                if ui.button(tr("Return to Painting Mode")).clicked() {
                    app.ui.mode = Mode::Painting;
                }
            });
        },
    );
}
