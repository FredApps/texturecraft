use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use serde_json::json;
use texturecraft_engine::core::BakedMap;

use crate::TextureCraftApp;
use crate::i18n::{baked_map_label, tr};

/// The texture set and selected map above Mesh Map Settings, each behind a dropdown to switch it
pub(super) fn show(app: &mut TextureCraftApp, ui: &mut Ui, selected: BakedMap) {
    let active_set = app.session.ts();
    let set_names: Vec<String> = app.session.doc.project.texture_sets.iter().map(|texture_set| texture_set.name.clone()).collect();

    ui.horizontal(|ui| {
        ui.menu_button(icon::CARET_RIGHT, |ui| {
            for (set_index, name) in set_names.iter().enumerate() {
                if ui.selectable_label(set_index == active_set, name).clicked() {
                    app.run("textureSet.select", json!({ "ts": set_index }));
                    ui.close();
                }
            }
        });
        ui.label(set_names.get(active_set).map_or("", String::as_str));

        ui.menu_button(icon::CARET_RIGHT, |ui| {
            for map in BakedMap::ALL {
                if ui.selectable_label(map == selected, tr(baked_map_label(map))).clicked() {
                    app.ui.selected_baked_map = Some(map);
                    ui.close();
                }
            }
        });
        ui.label(RichText::new(tr(baked_map_label(selected))).strong());
    });
}
