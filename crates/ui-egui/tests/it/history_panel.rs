//! A click on a step of the History panel goes back or forward to it.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

#[test]
fn clicking_a_history_step_returns_to_it() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = session.selected_layer().unwrap();
    for name in ["Step A", "Step B", "Step C"] {
        texturecraft_engine::execute(&mut session, "layer.rename", &json!({"layer": layer.0, "name": name})).unwrap();
    }
    let done = session.history_labels().len();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.dock.focus(Panel::History);
    h.run_steps(4);
    let name = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.layers[&layer].name.clone();
    // The first state: everything undone, and listed as undone.
    h.get_by_label("Project opened").click();
    h.run_steps(3);
    assert!(!h.state().session.can_undo());
    assert_eq!(h.state().session.redo_labels().len(), done);
    // The first undone row is the next step: one redo.
    h.get_all_by_label("Rename layer (undone)").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().session.history_labels().len(), 1);
    assert_eq!(h.state().session.redo_labels().len(), done - 1);
    // The last undone row: all the way forward again.
    h.get_all_by_label("Rename layer (undone)").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(name(&h), "Step C");
    assert!(!h.state().session.can_redo());
}
