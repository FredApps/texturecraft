//! Symmetry: the toolbar toggle and settings popup, and what the 3D view draws for it

mod ribbons;
mod settings_popup;
mod symmetry_display;
mod symmetry_drag;
mod symmetry_intersection;
mod symmetry_manipulator;
mod symmetry_manipulator_arrow;
mod view_overlay;

use egui::Ui;
use egui_phosphor::regular as ic;
use texturecraft_engine::Tool;
use texturecraft_engine::paint::Symmetry;

pub use symmetry_display::SymmetryDisplay;
pub(crate) use view_overlay::{manipulator_hovered, manipulator_input, overlay, scene_overlay};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::widgets::{icon_badge_dropdown_button, icon_button};

/// Radial copies a brush gets when it switches to radial symmetry
const DEFAULT_RADIAL_COUNT: u8 = 6;

/// Whether `tool` paints symmetry copies
pub(crate) fn applies_to(tool: Tool) -> bool {
    matches!(tool, Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Clone | Tool::Path)
}

/// Whether the 3D view shows symmetry now
pub(crate) fn shown(app: &TextureCraftApp) -> bool {
    app.ui.mode == crate::state::Mode::Painting && applies_to(app.session.tool.tool) && app.session.tool.brush.symmetry.any()
}

/// The axis the popup shows: the radial axis, or the first mirror axis
fn current_axis(symmetry: &Symmetry) -> usize {
    if symmetry.radial > 1 {
        return symmetry.axis_index();
    }
    [symmetry.x, symmetry.y, symmetry.z].iter().position(|on| *on).unwrap_or(symmetry.axis_index())
}

/// The world axes the plane (mirror) or the axis (radial) can move along
pub(crate) fn movable_axes(symmetry: &Symmetry) -> [bool; 3] {
    // The popup's type, also while the toggle is off
    if symmetry.radial > 1 {
        let axis = symmetry.axis_index();
        return [0, 1, 2].map(|other| other != axis);
    }
    [symmetry.x, symmetry.y, symmetry.z]
}

/// Mirror on one axis, or radial copies around it
fn set_mode(symmetry: &mut Symmetry, radial: bool, axis: usize) {
    symmetry.axis = axis as u8;
    if radial {
        (symmetry.x, symmetry.y, symmetry.z) = (false, false, false);
        if symmetry.radial <= 1 {
            symmetry.radial = DEFAULT_RADIAL_COUNT;
        }
    } else {
        (symmetry.x, symmetry.y, symmetry.z) = (axis == 0, axis == 1, axis == 2);
        symmetry.radial = 0;
    }
}

/// The toolbar's Symmetry toggle: turning it on with nothing set mirrors on X
fn toggle(symmetry: &mut Symmetry) {
    if symmetry.any() {
        symmetry.enabled = false;
        return;
    }

    symmetry.enabled = true;
    if !symmetry.x && !symmetry.y && !symmetry.z && symmetry.radial <= 1 {
        set_mode(symmetry, false, 0);
    }
}

/// The Symmetry toggle and the settings button with its popup
pub(crate) fn toolbar_controls(app: &mut TextureCraftApp, ui: &mut Ui) {
    let button_size = ui.spacing().interact_size.y;
    if icon_button(ui, ic::FLIP_HORIZONTAL, tr("Symmetry"), app.session.tool.brush.symmetry.any(), button_size).clicked() {
        toggle(&mut app.session.tool.brush.symmetry);
    }

    // No tooltip on the settings button; the label still names it for screen readers
    let popup_id = ui.make_persistent_id("symmetry_settings");
    let mut open = ui.data(|data| data.get_temp::<bool>(popup_id)).unwrap_or(false);
    let response = icon_badge_dropdown_button(ui, ic::FLIP_HORIZONTAL, ic::GEAR, tr("Symmetry settings"), open, button_size);
    if response.clicked() {
        open = !open;
    }

    // Open state kept here, not in egui's popup memory: that holds one popup, so the Type dropdown or the color picker inside would close this one
    let shown = egui::Popup::from_response(&response)
        .id(popup_id)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .show(|ui| settings_popup::show(app, ui));

    // A click outside the popup and the dropdowns it opens (foreground layers too) closes it
    if let Some(shown) = shown
        && !response.clicked()
        && ui.input(|input| input.pointer.any_click())
        && let Some(at) = ui.input(|input| input.pointer.interact_pos())
        && ui.ctx().layer_id_at(at).is_none_or(|layer| layer != shown.response.layer_id && layer.order != egui::Order::Foreground)
    {
        open = false;
    }
    ui.data_mut(|data| data.insert_temp(popup_id, open));
}
