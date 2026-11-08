//! The color picker popup keeps its place while the color changes, picks from its square and hue
//! strip, and copies its values.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::{theme, widgets};

/// A swatch at the right edge of the window, like the Properties panel's, with its picker open.
fn open_picker(color: [f32; 3]) -> Harness<'static, [f32; 3]> {
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, color: &mut [f32; 3]| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                widgets::color_swatch(ui, "color", color, 120.0);
            });
        },
        color,
    );
    h.ctx.set_fonts(theme::font_definitions());
    theme::apply(&h.ctx, theme::Brightness::default(), theme::Accent::default());
    h.run_steps(2);
    let swatch = egui::pos2(800.0 - 60.0, 9.0);
    h.drag_at(swatch);
    h.drop_at(swatch);
    h.run_steps(3);
    h
}

/// The picker's R value.
fn r_value(h: &Harness<'_, [f32; 3]>) -> egui::Rect {
    h.get_all_by_role(Role::SpinButton).next().unwrap().rect()
}

/// Click at `p` and let the picker apply it.
fn click(h: &mut Harness<'_, [f32; 3]>, p: egui::Pos2) {
    h.drag_at(p);
    h.drop_at(p);
    h.run_steps(3);
}

#[test]
fn the_popup_stays_put_while_the_color_changes() {
    let mut h = open_picker([1.0; 3]);
    let bright = r_value(&h).left();
    *h.state_mut() = [0.001; 3];
    h.run_steps(3);
    assert_eq!(r_value(&h).left(), bright, "the picker does not move when R, G and B lose digits");
}

#[test]
fn the_square_and_the_hue_strip_pick_the_color() {
    let mut h = open_picker([1.0; 3]);
    // The square starts a row below the values; its bottom-left corner is black.
    let r = r_value(&h);
    let square = egui::Rect::from_min_size(egui::pos2(r.left() + 1.0, r.bottom() + 8.0), egui::vec2(273.0, 273.0));
    click(&mut h, square.left_bottom() + egui::vec2(2.0, -6.0));
    assert!(h.state().iter().all(|c| *c < 0.01), "black: {:?}", h.state());
    // Full saturation and value, then a hue a third of the way along: green.
    click(&mut h, square.right_top() + egui::vec2(-1.0, 1.0));
    let strip = square.bottom() + 14.0;
    click(&mut h, egui::pos2(square.left() + 275.0 / 3.0, strip));
    let [r, g, b] = *h.state();
    assert!(g > 0.9 && r < 0.05 && b < 0.05, "green: {:?}", h.state());
}

#[test]
fn the_copy_button_copies_the_rgb_values() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    h.get_by_label("Copy RGB values").click();
    h.step();
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "255, 0, 0"));
    assert!(copied, "{:?}", h.output().platform_output.commands);
}
