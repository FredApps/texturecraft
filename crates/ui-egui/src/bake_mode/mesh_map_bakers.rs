use egui::{Align, Layout, RichText, Sense, Stroke, Ui, UiBuilder, vec2};
use egui_phosphor::regular as icon;
use texturecraft_engine::core::{BakeSettings, BakedMap};

use crate::TextureCraftApp;
use crate::docking::Panel;
use crate::i18n::{baked_map_label, tr};
use crate::theme::Tokens;

/// Height of one baker row, in points
const ROW_HEIGHT: f32 = 24.0;
/// Inset of a row's checkbox and name from its left edge, in points
const ROW_INSET: f32 = 6.0;

/// The Common Settings button, Check All, and one row per map (checkbox enables, click selects)
pub(super) fn show(app: &mut TextureCraftApp, ui: &mut Ui, settings: &mut BakeSettings) {
    if ui.button(format!("{} {}", icon::GEAR, tr("Common Settings"))).clicked() {
        app.ui.dock.focus(Panel::CommonSettings);
    }

    ui.add_space(4.0);
    check_all(ui, settings);
    ui.separator();

    let selected = super::selected_map(app.ui.selected_baked_map, settings);
    for map in BakedMap::ALL {
        if ui.push_id(map, |ui| row(ui, map, settings, map == selected)).inner {
            app.ui.selected_baked_map = Some(map);
            app.ui.dock.focus(Panel::MeshMapSettings);
        }
    }
}

/// Enable every map, or disable them all when every map is already enabled
fn check_all(ui: &mut Ui, settings: &mut BakeSettings) {
    let enabled_count = BakedMap::ALL.into_iter().filter(|map| settings.maps.is_enabled(*map)).count();
    let mut all_enabled = enabled_count == BakedMap::ALL.len();
    let some_enabled = enabled_count > 0 && !all_enabled;

    if ui.add(egui::Checkbox::new(&mut all_enabled, tr("Check All")).indeterminate(some_enabled)).changed() {
        for map in BakedMap::ALL {
            settings.maps.set_enabled(map, all_enabled);
        }
    }
}

/// One baker row; returns whether the row (not its checkbox) was clicked
fn row(ui: &mut Ui, map: BakedMap, settings: &mut BakeSettings, is_selected: bool) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let name = tr(baked_map_label(map));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, is_selected, name));

    let painter = ui.painter();
    if is_selected {
        painter.rect_filled(rect, tokens.cr(), tokens.row_selected);
        painter.rect_stroke(rect, tokens.cr(), Stroke::new(1.0, tokens.accent), egui::StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(rect, tokens.cr(), tokens.row_hover);
    }
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, tokens.separator));

    // Added after the row's own response, so the checkbox takes the clicks over it
    let content_rect = rect.shrink2(vec2(ROW_INSET, 0.0));
    let mut content = ui.new_child(UiBuilder::new().max_rect(content_rect).layout(Layout::left_to_right(Align::Center)));
    let mut is_enabled = settings.maps.is_enabled(map);
    let checkbox = content.checkbox(&mut is_enabled, "");
    let text = if is_selected { RichText::new(name).strong() } else { RichText::new(name) };
    let label = content.add(egui::Label::new(text).selectable(false).truncate());
    if checkbox.labelled_by(label.id).changed() {
        settings.maps.set_enabled(map, is_enabled);
    }

    response.clicked()
}
