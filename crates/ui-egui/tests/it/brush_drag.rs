//! Ctrl+drag in the 3D view changes the brush instead of painting or orbiting.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;

/// Drag with `button` from `from` by `by` in a few moves.
fn drag(h: &mut Harness<'_, TextureCraftApp>, button: PointerButton, from: Pos2, by: egui::Vec2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(from);
    h.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    h.run_steps(2);
    for k in 1..=4 {
        h.hover_at(from + by * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: from + by, button, pressed: false, modifiers });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn ctrl_drag_changes_the_brush_instead_of_painting_or_orbiting() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let c = h.state().viewport_rect.center();
    let (brush, yaw, history) = {
        let s = &h.state().session;
        (s.tool.brush.clone(), s.camera.yaw, s.history_labels().len())
    };
    // Right drag to the right and up: a bigger, harder brush; the camera stays.
    drag(&mut h, PointerButton::Secondary, c, vec2(80.0, -40.0), Modifiers::COMMAND);
    let b = h.state().session.tool.brush.clone();
    assert!(b.size > brush.size && b.hardness > brush.hardness, "{} {} → {} {}", brush.size, brush.hardness, b.size, b.hardness);
    assert_eq!(h.state().session.camera.yaw, yaw, "no orbit");
    // Left drag to the left and up: less flow, a turned brush, and no stroke.
    drag(&mut h, PointerButton::Primary, c, vec2(-80.0, -40.0), Modifiers::COMMAND);
    let b2 = h.state().session.tool.brush.clone();
    assert!(b2.flow < b.flow && b2.angle != b.angle, "{} {} → {} {}", b.flow, b.angle, b2.flow, b2.angle);
    assert_eq!(h.state().session.history_labels().len(), history, "nothing painted");
    // Without Ctrl, a right drag orbits again and the brush keeps its values.
    drag(&mut h, PointerButton::Secondary, c, vec2(80.0, -40.0), Modifiers::NONE);
    assert_ne!(h.state().session.camera.yaw, yaw);
    assert_eq!(h.state().session.tool.brush, b2);
}
