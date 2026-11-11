//! A plain right click in a view opens the Properties panel's contents at the cursor; right drags keep their gestures

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::LayerKind;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::properties_popup::{self, PropertiesPopup};
use texturecraft_ui_egui::state::{Mode, ViewLayout};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness
}

fn view_center(harness: &Harness<'_, TextureCraftApp>) -> Pos2 {
    harness.state().viewport_rect.center()
}

fn is_open(harness: &Harness<'_, TextureCraftApp>) -> bool {
    matches!(harness.state().properties_popup, Some(PropertiesPopup::Open { .. }))
}

/// Press and release `button` at `at` without moving
fn click(harness: &mut Harness<'_, TextureCraftApp>, button: PointerButton, at: Pos2) {
    harness.hover_at(at);
    harness.event(Event::PointerButton { pos: at, button, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: at, button, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
}

/// Drag with `button` from `from` by `by` in a few moves while `modifiers` are held
fn drag(harness: &mut Harness<'_, TextureCraftApp>, button: PointerButton, from: Pos2, by: egui::Vec2, modifiers: Modifiers) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
    harness.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    harness.run_steps(2);
    for step in 1..=4 {
        harness.hover_at(from + by * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: from + by, button, pressed: false, modifiers });
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(3);
}

#[test]
fn a_right_click_opens_the_popup_at_the_cursor_in_both_views() {
    let mut harness = harness();
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    assert!(matches!(harness.state().properties_popup, Some(PropertiesPopup::Open { at, .. }) if at == center));

    harness.key_press(Key::Escape);
    harness.run_steps(2);
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(2);
    click(&mut harness, PointerButton::Secondary, center);
    assert!(is_open(&harness), "the 2D view opens it too");
}

#[test]
fn right_drags_keep_their_gestures_and_never_open_it() {
    let mut harness = harness();
    let center = view_center(&harness);
    let yaw = harness.state().session.camera.yaw;
    drag(&mut harness, PointerButton::Secondary, center, vec2(80.0, -40.0), Modifiers::NONE);
    assert_ne!(harness.state().session.camera.yaw, yaw, "a right drag orbits");
    assert!(!is_open(&harness), "an orbit doesn't open it");

    let size = harness.state().session.tool.brush.size;
    drag(&mut harness, PointerButton::Secondary, center, vec2(80.0, -40.0), Modifiers::COMMAND);
    assert!(harness.state().session.tool.brush.size > size, "Ctrl+right drag sets the brush");
    assert!(!is_open(&harness));

    for modifiers in [Modifiers::ALT, Modifiers::SHIFT] {
        drag(&mut harness, PointerButton::Secondary, center, vec2(80.0, -40.0), modifiers);
        assert!(!is_open(&harness), "{modifiers:?}");
    }
}

#[test]
fn a_click_outside_closes_it_without_painting() {
    let mut harness = harness();
    let center = view_center(&harness);
    let outside = harness.state().viewport_rect.left_center() + vec2(20.0, 0.0);
    let history = harness.state().session.history_labels().len();
    click(&mut harness, PointerButton::Secondary, center);
    assert!(is_open(&harness));

    click(&mut harness, PointerButton::Primary, outside);
    assert_eq!(harness.state().properties_popup, None);
    assert_eq!(harness.state().session.history_labels().len(), history, "the closing click doesn't paint");

    click(&mut harness, PointerButton::Secondary, center);
    click(&mut harness, PointerButton::Secondary, outside);
    assert_eq!(harness.state().properties_popup, None, "a right click outside closes it and doesn't reopen it");
}

#[test]
fn escape_closes_it() {
    let mut harness = harness();
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    assert!(is_open(&harness));
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().properties_popup, None);
}

#[test]
fn dragging_its_size_slider_changes_the_brush() {
    let mut harness = harness();
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();

    // The Size section's header sits above its slider of the same name
    let slider_label = harness
        .query_all_by_label("Size")
        .map(|node| node.rect())
        .filter(|rect| popup.contains_rect(*rect))
        .max_by(|first, second| first.top().total_cmp(&second.top()))
        .unwrap();
    let track_y = slider_label.bottom() + 10.0;
    let size = harness.state().session.tool.brush.size;
    harness.drag_at(egui::pos2(slider_label.left() + 20.0, track_y));
    harness.run_steps(2);
    harness.hover_at(egui::pos2(popup.right() - 30.0, track_y));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(popup.right() - 30.0, track_y));
    harness.run_steps(3);
    assert!(harness.state().session.tool.brush.size > size, "{size} -> {}", harness.state().session.tool.brush.size);
    assert!(is_open(&harness), "editing it keeps it open");
}

#[test]
fn it_shows_what_the_properties_panel_shows() {
    let mut harness = harness();
    let fill = harness.state().session.doc.project.layers.values().find(|layer| matches!(layer.kind, LayerKind::Fill(_))).map(|layer| layer.id.0).unwrap();
    let selected = texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": fill}));
    assert!(selected.is_ok(), "{selected:?}");
    harness.run_steps(3);

    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();
    let in_popup = |label: &str| harness.query_all_by_label(label).any(|node| popup.contains_rect(node.rect()));
    assert!(in_popup("Projection"), "a selected fill shows its fill settings");
    assert!(!in_popup("Minimum size (%)"), "not the brush's");
}

#[test]
fn bake_and_render_modes_never_open_it() {
    let mut harness = harness();
    let center = view_center(&harness);
    for mode in [Mode::Baking, Mode::Rendering] {
        harness.state_mut().ui.mode = mode;
        harness.run_steps(3);
        click(&mut harness, PointerButton::Secondary, center);
        assert_eq!(harness.state().properties_popup, None, "{mode:?}");
    }
}
