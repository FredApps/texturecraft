//! Polygon fill in the views: a click fills one target and a box drag fills what the box selects, each as one undo step

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use texturecraft_engine::Tool;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;

/// The sample project in the default split layout at 1600 x 960 with the polygon fill tool
fn polygon_fill_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().session.tool.tool = Tool::PolygonFill;
    harness.run_steps(4);
    harness
}

/// Drag the primary button from `from` to `to` in a few moves
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run_steps(1);
    }

    // Only the release frame runs, so the release itself must finish the drag
    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(1);
}

/// Press and release the primary button at `at`
fn click(harness: &mut Harness<'_, TextureCraftApp>, at: Pos2) {
    harness.hover_at(at);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
}

/// Allocated tiles in the selected layer's base color
fn painted_tiles(harness: &Harness<'_, TextureCraftApp>) -> usize {
    let session = &harness.state().session;
    let layer = session.selected_layer().expect("a selected layer");
    session.doc.pixels.layer(layer, Channel::BaseColor).map_or(0, |image| image.allocated_count())
}

/// The undo steps recorded so far
fn history(harness: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    harness.state().session.history_labels()
}

#[test]
fn a_box_drag_in_the_3d_view_is_one_fill_over_more_than_a_click() {
    let mut clicked = polygon_fill_app();
    let center = clicked.state().viewport_rect.center();
    let before = history(&clicked).len();
    click(&mut clicked, center);
    assert_eq!(history(&clicked).len(), before + 1, "a click fills one triangle");

    let mut boxed = polygon_fill_app();
    drag(&mut boxed, center - vec2(120.0, 120.0), center + vec2(120.0, 120.0));
    assert_eq!(history(&boxed).len(), before + 1);
    assert_eq!(history(&boxed).last().map(String::as_str), Some("Polygon fill"));
    assert!(
        painted_tiles(&boxed) > painted_tiles(&clicked),
        "the box fills every triangle in it: {} tiles vs {}",
        painted_tiles(&boxed),
        painted_tiles(&clicked)
    );
}

#[test]
fn a_box_drag_in_the_2d_view_is_one_fill() {
    let mut harness = polygon_fill_app();
    let view_3d = harness.state().viewport_rect;
    let center = pos2(view_3d.right() + view_3d.width() * 0.5, view_3d.center().y);
    let before = history(&harness).len();

    drag(&mut harness, center - vec2(80.0, 20.0), center + vec2(80.0, 20.0));
    assert_eq!(history(&harness).len(), before + 1);
    assert_eq!(history(&harness).last().map(String::as_str), Some("Polygon fill"));
}

#[test]
fn a_new_press_after_release_does_not_resume_the_previous_box() {
    let mut harness = polygon_fill_app();
    let center = harness.state().viewport_rect.center();
    drag(&mut harness, center - vec2(40.0, 40.0), center + vec2(40.0, 40.0));
    let history_after_drag = history(&harness);
    let tiles_after_drag = painted_tiles(&harness);

    // Start the next gesture on the frame immediately after release
    let point = center - vec2(80.0, 80.0);
    harness
        .input_mut()
        .events
        .extend([Event::PointerMoved(point), Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    harness.run_steps(1);
    assert_eq!(painted_tiles(&harness), tiles_after_drag, "a press alone must not refill the old rectangle");

    harness.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    assert_eq!(history(&harness).len(), history_after_drag.len() + 1, "only the new click records a step");
}

#[test]
#[ignore = "requires an offscreen GPU adapter and TC_UI_SNAPSHOTS"]
fn polygon_fill_snapshots() {
    let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else { return };

    std::fs::create_dir_all(&directory).unwrap();
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(vec2(1600.0, 960.0)).wgpu().build_eframe(|context| TextureCraftApp::new(context, session));
    let adapter = harness.state().render_state.as_ref().unwrap().adapter.get_info();
    eprintln!("Offscreen adapter: {} ({:?}), driver {} {}", adapter.name, adapter.backend, adapter.driver, adapter.driver_info);
    harness.state_mut().session.tool.tool = Tool::PolygonFill;
    harness.run_steps(6);
    harness.render().unwrap().save(directory.join("polygon-fill-properties.png")).unwrap();

    let center = harness.state().viewport_rect.center();
    let origin = center - vec2(85.0, 70.0);
    let opposite = center + vec2(85.0, 70.0);
    harness.hover_at(origin);
    harness.event(Event::PointerButton { pos: origin, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    harness.hover_at(opposite);
    harness.run_steps(6);
    harness.render().unwrap().save(directory.join("polygon-fill-drag-3d.png")).unwrap();
    harness.event(Event::PointerButton { pos: opposite, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(6);
    harness.render().unwrap().save(directory.join("polygon-fill-complete-3d.png")).unwrap();

    texturecraft_engine::execute(&mut harness.state_mut().session, "mask.add.black", &serde_json::Value::Null).unwrap();
    let view = harness.state().viewport_rect;
    let center = pos2(view.right() + view.width() * 0.5, view.center().y);
    drag(&mut harness, center - vec2(80.0, 20.0), center + vec2(80.0, 20.0));
    harness.run_steps(6);
    harness.render().unwrap().save(directory.join("polygon-fill-mask-2d.png")).unwrap();
}
