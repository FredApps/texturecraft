//! Pick mode of a color selection: the next click in the 3D or 2D view adds the ID color under it

use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, LayerId};
use texturecraft_engine::{StackTarget, ViewMode};

use crate::TextureCraftApp;
use crate::state::Mode;

/// Height in points of the eyedropper drawn as the pointer while picking
const EYEDROPPER_SIZE: f32 = 20.0;

/// A color selection waiting for a click, and the view to return to once it comes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdColorPick {
    pub effect: LayerId,
    /// The color the click replaces; None adds one
    pub index: Option<usize>,
    previous_view: ViewMode,
}

impl IdColorPick {
    /// Waits for a click that picks into `effect`, showing the ID map meanwhile
    pub fn start(app: &mut TextureCraftApp, effect: LayerId, index: Option<usize>) {
        let previous_view = match app.picking_id_color.take() {
            Some(running) => running.previous_view,
            None => app.session.view,
        };

        app.properties_popup = None;
        app.picking_id_color = Some(IdColorPick { effect, index, previous_view });
        app.run("viewport.view", json!({"mode": format!("map:{}", BakedMap::Id.id())}));
    }

    /// Leaves pick mode, returning to the earlier view unless the user chose another meanwhile
    pub fn end(app: &mut TextureCraftApp) {
        let Some(pick) = app.picking_id_color.take() else { return };

        if app.session.view == ViewMode::MeshMap(BakedMap::Id) {
            app.run("viewport.view", view_params(pick.previous_view));
        }
    }

    /// Picks at `point` (`x`/`y`/`viewport` or `uv` params) and leaves pick mode
    pub fn pick_at(app: &mut TextureCraftApp, point: Value) {
        let Some(pick) = app.picking_id_color else { return };

        let mut params = json!({"effect": pick.effect.0, "index": pick.index});
        if let (Some(target), Some(source)) = (params.as_object_mut(), point.as_object()) {
            target.extend(source.clone());
        }

        app.run("effect.colorSelection.pick", params);
        IdColorPick::end(app);
    }

    /// An eyedropper in place of the pointer while it is over `rect`, its tip on the hotspot
    pub fn draw_cursor(ui: &egui::Ui, rect: egui::Rect) {
        let Some(pointer) = ui.ctx().pointer_hover_pos().filter(|pointer| rect.contains(*pointer)) else { return };

        ui.ctx().set_cursor_icon(egui::CursorIcon::None);

        // The glyph's tip is its bottom-left corner; a dark outline keeps it readable on any ID color
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("id_color_pick_cursor")));
        let font = egui::FontId::proportional(EYEDROPPER_SIZE);
        let glyph = egui_phosphor::regular::EYEDROPPER;
        for offset in [egui::vec2(-1.0, 0.0), egui::vec2(1.0, 0.0), egui::vec2(0.0, -1.0), egui::vec2(0.0, 1.0)] {
            painter.text(pointer + offset, egui::Align2::LEFT_BOTTOM, glyph, font.clone(), egui::Color32::from_black_alpha(220));
        }
        painter.text(pointer, egui::Align2::LEFT_BOTTOM, glyph, font, egui::Color32::WHITE);
    }

    /// Ends pick mode once its effect is no longer selected or painting mode is left
    pub fn end_when_stale(app: &mut TextureCraftApp) {
        let Some(pick) = app.picking_id_color else { return };

        if app.session.target != StackTarget::Effect(pick.effect) || app.ui.mode != Mode::Painting {
            IdColorPick::end(app);
        }
    }
}

/// The `viewport.view` params that show `view`
fn view_params(view: ViewMode) -> Value {
    match view {
        ViewMode::Material => json!({"mode": "material"}),
        ViewMode::Channel(channel) => json!({"mode": channel.id()}),
        ViewMode::MeshMap(map) => json!({"mode": format!("map:{}", map.id())}),
        ViewMode::UvChecker => json!({"mode": "uvChecker"}),
        ViewMode::Mask(layer) => json!({"mode": "mask", "layer": layer.0}),
    }
}
