//! The Path tool in the 2D view: placing previews the path, and points are edited where they are drawn

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

type AppHarness<'a> = Harness<'a, TextureCraftApp>;

// Three points left to right across the middle of the 2D view's texture
const A: Pos2 = pos2(600.0, 500.0);
const B: Pos2 = pos2(700.0, 500.0);
const C: Pos2 = pos2(800.0, 500.0);

fn harness() -> AppHarness<'static> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.paint", &serde_json::Value::Null).unwrap();
    execute(&mut session, "tool.select", &json!({"tool": "path"})).unwrap();
    // Tall enough that the 2D view's texture square takes A, B and C
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.layout = ViewLayout::Only2D;
    h.run_steps(4);
    h
}

fn pointer(h: &mut AppHarness<'_>, pos: Pos2, pressed: Option<bool>) {
    let events = &mut h.input_mut().events;
    events.push(Event::PointerMoved(pos));
    if let Some(pressed) = pressed {
        events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
}

fn click(h: &mut AppHarness<'_>, at: Pos2) {
    pointer(h, at, Some(true));
    pointer(h, at, Some(false));
    h.run_steps(2);
}

fn drag(h: &mut AppHarness<'_>, from: Pos2, to: Pos2) {
    pointer(h, from, Some(true));
    for k in 1..=6 {
        pointer(h, from + (to - from) * (k as f32 / 6.0), None);
    }
    pointer(h, to, Some(false));
    h.run_steps(2);
}

fn key(h: &mut AppHarness<'_>, key: Key) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// The path sublayers of the selected layer: (id, stored paths)
fn path_sublayers(h: &AppHarness<'_>) -> Vec<(LayerId, usize)> {
    let s = &h.state().session;
    let layer = s.doc.project.layer(s.selected_layer().unwrap()).unwrap();
    layer.content_effects.iter().filter_map(|e| s.paths(e.id).map(|p| (e.id, p.len()))).collect()
}

/// Channel images painted in path sublayer `eid`
fn painted_channels(h: &AppHarness<'_>, eid: LayerId) -> usize {
    h.state().session.doc.pixels.layers.iter().filter(|((id, _), _)| *id == eid).count()
}

#[test]
fn placing_points_previews_the_path_and_enter_applies_it_as_one_step() {
    let mut h = harness();
    let steps = h.state().session.history_labels().len();
    for at in [A, B, C] {
        click(&mut h, at);
    }
    assert_eq!(h.state().session.tool.path.points.len(), 3);
    let [(eid, 1)] = path_sublayers(&h)[..] else { panic!("the preview is a path in a path sublayer: {:?}", path_sublayers(&h)) };
    assert!(painted_channels(&h, eid) > 0, "the preview painted");
    assert_eq!(h.state().session.history_labels().len(), steps, "no undo step while placing");
    key(&mut h, Key::Enter);
    assert_eq!(h.state().session.history_labels().len(), steps + 1);
    assert_eq!(h.state().session.undo_label(), Some("Path"));
    assert!(h.state().session.tool.path.points.is_empty());
    assert_eq!(path_sublayers(&h), vec![(eid, 1)]);
}

#[test]
fn clicking_an_applied_path_edits_it_and_dragging_a_point_changes_it() {
    let mut h = harness();
    for at in [A, B, C] {
        click(&mut h, at);
    }
    key(&mut h, Key::Enter);
    let [(eid, 1)] = path_sublayers(&h)[..] else { panic!("one path") };
    let stored = |h: &AppHarness<'_>| h.state().session.paths(eid).unwrap()[0].points.clone();
    let before = stored(&h);
    let steps = h.state().session.history_labels().len();

    // A click on the path's curve opens it for editing without repainting it
    click(&mut h, A.lerp(B, 0.5));
    assert_eq!(h.state().session.tool.path.editing, Some((eid, 0)));
    assert_eq!(h.state().session.history_labels().len(), steps, "opening a path repaints nothing");

    drag(&mut h, B, B + egui::vec2(0.0, 60.0));
    let after = stored(&h);
    assert_eq!(after.len(), 3);
    assert_eq!((after[0], after[2]), (before[0], before[2]));
    assert!(after[1][1] > before[1][1], "the dragged point moved down: {:?} -> {:?}", before[1], after[1]);
    assert_eq!(h.state().session.history_labels().len(), steps + 1, "the edit is one step");
}

#[test]
fn clicking_the_curve_inserts_a_point_and_backspace_removes_the_selected_one() {
    let mut h = harness();
    for at in [A, B, C] {
        click(&mut h, at);
    }
    let path = |h: &AppHarness<'_>| h.state().session.tool.path.clone();
    let before = path(&h).points;
    click(&mut h, B.lerp(C, 0.5));
    let after = path(&h);
    assert_eq!(after.points.len(), 4, "inserted, not appended");
    assert_eq!(after.selected, Some(2));
    assert_eq!((after.points[1], after.points[3]), (before[1], before[2]));
    assert!(after.points[2].x > before[1].x && after.points[2].x < before[2].x);

    // B, not A: clicking the first point closes the path
    click(&mut h, B);
    assert_eq!(path(&h).selected, Some(1));
    key(&mut h, Key::Backspace);
    assert_eq!(path(&h).points, vec![after.points[0], after.points[2], after.points[3]]);

    // Undo brings the point back and keeps the path; again, the inserted point goes
    let undo = |h: &mut AppHarness<'_>| {
        h.input_mut().events.push(Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
        h.run_steps(2);
    };
    undo(&mut h);
    assert_eq!(path(&h).points, after.points);
    undo(&mut h);
    assert_eq!(path(&h).points, before);
    assert!(!path_sublayers(&h).is_empty(), "the path is still there");
}
