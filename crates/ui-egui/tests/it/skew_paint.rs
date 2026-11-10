//! Bake mode: with the skew brush on, dragging in the 2D view paints the skew map, and the
//! brush controls are in Common Settings.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{Mode, ViewLayout};

/// A bake-mode app with a high poly, the 2D view, and Common Settings in front
fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.add_high_poly_bytes("High.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.mode = Mode::Baking;
    h.state_mut().ui.layout = ViewLayout::Only2D;
    h.run_steps(4);
    h.state_mut().ui.dock.focus(texturecraft_ui_egui::docking::Panel::CommonSettings);
    h.run_steps(3);
    h
}

#[test]
fn the_bake_panel_has_the_bake_paint_brush_and_dragging_paints_the_map() {
    let mut h = harness();
    h.get_by_label("Skew Correction");
    assert!(h.state().session.doc.bake_paint.is_empty() && !h.state().ui.bake_paint_brush.on);
    // Off: a drag in the view paints nothing.
    h.drag_at(egui::pos2(600.0, 500.0));
    h.hover_at(egui::pos2(800.0, 500.0));
    h.drop_at(egui::pos2(800.0, 500.0));
    h.run_steps(3);
    assert!(h.state().session.doc.bake_paint.is_empty(), "the brush is off");

    // On: the checkbox turns it on, and the same drag makes a map with paint along the stroke.
    h.get_by_label("Paint Skew Correction").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Paint Skew Correction").click();
    h.run_steps(3);
    assert!(h.state().ui.bake_paint_brush.on);
    h.get_by_label("Radius");
    h.drag_at(egui::pos2(600.0, 500.0));
    h.run_steps(2);
    h.hover_at(egui::pos2(800.0, 500.0));
    h.run_steps(2);
    h.drop_at(egui::pos2(800.0, 500.0));
    h.run_steps(3);
    let map = h.state().session.doc.bake_paint.get(&(0, texturecraft_engine::core::BakePaintTarget::Skew)).cloned().expect("the stroke made a skew map");
    let painted = map.values.iter().filter(|v| **v > 0.5).count();
    assert!(painted > 50 && painted < map.values.len() / 2, "a stroke, not the whole map: {painted} of {}", map.values.len());

    // Removing it from the panel clears the map.
    h.get_by_label("Remove Skew Correction").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Remove Skew Correction").click();
    h.run_steps(3);
    assert!(h.state().session.doc.bake_paint.is_empty());
}
