use egui::collapsing_header::CollapsingState;
use egui::{Rect, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets::{icon_button_without_tooltip, section_chevron};

/// The panel's width and its gap from the 3D view's top-left corner
const PANEL_WIDTH: f32 = 280.0;
const PANEL_MARGIN: f32 = 12.0;

/// Size of the panel's eye buttons
const EYE_BUTTON_SIZE: f32 = 22.0;

/// The Baking Visualization panel in the top-left corner of bake mode's 3D view `view`
pub(crate) fn bake_visualization_panel(app: &mut TextureCraftApp, ui: &Ui, view: Rect) {
    let toggles = &mut app.ui.bake_visualization;
    egui::Area::new(ui.scope_id().with("bake_visualization_panel")).fixed_pos(view.left_top() + vec2(PANEL_MARGIN, PANEL_MARGIN)).constrain_to(view).show(
        ui.ctx(),
        |ui| {
            ui.set_width(PANEL_WIDTH);
            let mut collapsing_state = CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id("bake_visualization_open"), true);
            let header = eye_row(ui, &mut toggles.on, tr("Baking Visualization"), Some(collapsing_state.is_open()));
            if header.clicked() {
                collapsing_state.toggle(ui);
            }

            collapsing_state.show_body_unindented(ui, |ui| {
                ui.add_enabled_ui(toggles.on, |ui| {
                    ui.checkbox(&mut toggles.selected_set_only, tr("Show for Selected Texture Set Only"));
                    eye_row(ui, &mut toggles.high_poly, tr("High Definition Mesh (HP)"), None);
                    eye_row(ui, &mut toggles.cage, tr("Cage"), None);
                    eye_row(ui, &mut toggles.uv_seams, tr("UV Seams"), None);
                    eye_row(ui, &mut toggles.low_poly, tr("Project Mesh (LP)"), None);
                });
            });
        },
    );
}

/// A row with `label`, a chevron when `open` is given, and an eye button on the right that switches `shown`; returns the label's response
fn eye_row(ui: &mut Ui, shown: &mut bool, label: &str, open: Option<bool>) -> egui::Response {
    let tokens = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.set_min_height(EYE_BUTTON_SIZE);
        let text = match open {
            Some(is_open) => egui::RichText::new(format!("{}  {label}", section_chevron(ui, is_open).text())),
            None => egui::RichText::new(label),
        };
        let response = ui.add(egui::Label::new(text.color(tokens.text_strong)).selectable(false).sense(egui::Sense::click()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button_without_tooltip(ui, if *shown { icon::EYE } else { icon::EYE_SLASH }, label, false, EYE_BUTTON_SIZE).clicked() {
                *shown = !*shown;
            }
        });
        response
    })
    .inner
}
