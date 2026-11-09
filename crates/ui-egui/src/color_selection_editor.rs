//! Properties of a color selection effect: tolerance and the ID colors it selects

use egui::{Color32, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::{BakedMap, LayerId};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::id_color_pick::IdColorPick;
use crate::theme::Tokens;
use crate::widgets::{icon_button, slider_edit};

/// Width of one R, G or B readout, so the columns line up from row to row (fits "G: 255")
const READOUT_WIDTH: f32 = 50.0;

/// Height of a color row's swatch and readouts, as tall as the app's other swatches
const ROW_HEIGHT: f32 = 18.0;

const SWATCH_WIDTH: f32 = 36.0;

const ROW_BUTTON_SIZE: f32 = 20.0;

/// Tolerance, a Colors list with a pick mode, and one row per ID color (swatch, re-pick, R/G/B, remove)
pub fn color_selection_properties(app: &mut TextureCraftApp, ui: &mut Ui, effect: LayerId, colors: &[[f32; 3]], tolerance: f32) {
    let tokens = Tokens::get(ui.ctx());
    let mut edited_tolerance = tolerance;
    if let Some(merge) = slider_edit(ui, tr("Tolerance"), &mut edited_tolerance, 0.0..=1.0, 2) {
        app.run("effect.set", json!({"effect": effect.0, "tolerance": edited_tolerance, "merge": merge}));
    }

    let set_index = app.session.doc.project.locate(effect_owner(app, effect)).map_or(app.session.ts(), |(set_index, _, _)| set_index);
    let has_id_map = app.session.baked(set_index).is_some_and(|baked| baked.maps.contains_key(&BakedMap::Id));
    if !has_id_map {
        ui.label(RichText::new(format!("{} {}", ic::WARNING, tr("Bake an ID map (F8) to pick colors from the mesh."))).color(tokens.warning));
    }

    let picking = app.picking_id_color.filter(|pick| pick.effect == effect);
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label(tr("Colors"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let adding = picking.is_some_and(|pick| pick.index.is_none());
                let button = egui::Button::new(format!("{} {}", ic::EYEDROPPER, tr("Pick color"))).selected(adding);
                if ui.add_enabled(has_id_map, button).clicked() {
                    toggle_pick(app, effect, None, adding);
                }
            });
        });

        for (index, color) in colors.iter().enumerate() {
            ui.horizontal(|ui| {
                id_swatch(ui, *color);
                let repicking = picking.is_some_and(|pick| pick.index == Some(index));
                let repick = ui.add_enabled_ui(has_id_map, |ui| icon_button(ui, ic::EYEDROPPER, tr("Pick color"), repicking, ROW_BUTTON_SIZE)).inner;
                if repick.clicked() {
                    toggle_pick(app, effect, Some(index), repicking);
                }

                for (channel, value) in ["R", "G", "B"].iter().zip(display_bytes(*color)) {
                    ui.allocate_ui_with_layout(vec2(READOUT_WIDTH, ROW_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(READOUT_WIDTH);
                        ui.label(format!("{channel}: {value}"));
                    });
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let remove = ui.add_enabled_ui(colors.len() > 1, |ui| icon_button(ui, ic::MINUS, tr("Remove"), false, ROW_BUTTON_SIZE)).inner;
                    if remove.clicked() {
                        // Removing a row changes the indices of pending replacements
                        IdColorPick::end(app);
                        let remaining: Vec<[f32; 3]> = colors.iter().enumerate().filter(|(kept, _)| *kept != index).map(|(_, kept)| *kept).collect();
                        app.run("effect.set", json!({"effect": effect.0, "colors": remaining}));
                    }
                });
            });
        }
    });
}

/// The layer whose mask holds `effect`
fn effect_owner(app: &TextureCraftApp, effect: LayerId) -> LayerId {
    app.session.doc.project.effect(effect).map_or(effect, |(owner, _)| owner)
}

fn toggle_pick(app: &mut TextureCraftApp, effect: LayerId, index: Option<usize>, running: bool) {
    if running {
        IdColorPick::end(app);
    } else {
        IdColorPick::start(app, effect, index);
    }
}

/// An ID color as the ID map view draws it: the stored value, unconverted, in 0–255
fn display_bytes(color: [f32; 3]) -> [u8; 3] {
    color.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn id_swatch(ui: &mut Ui, color: [f32; 3]) {
    let tokens = Tokens::get(ui.ctx());
    let [red, green, blue] = display_bytes(color);
    let (rect, _) = ui.allocate_exact_size(vec2(SWATCH_WIDTH, ROW_HEIGHT), Sense::hover());
    ui.painter().rect(rect, f32::from(tokens.radius), Color32::from_rgb(red, green, blue), Stroke::new(1.0, tokens.field_border), egui::StrokeKind::Inside);
}
