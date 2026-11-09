//! The start window: New project opens its dialog, Continue keeps the open project, and the
//! startup choice is a setting.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.windows.start = true;
    h.run_steps(3);
    h
}

#[test]
fn start_window_choices() {
    let mut h = harness();
    h.get_by_label("New project...").click();
    h.run_steps(3);
    assert!(!h.state().ui.windows.start && h.state().ui.windows.new_project, "New project opens its dialog");

    let mut h = harness();
    h.get_by_label("Show at startup").click();
    h.get_by_label("Continue with the sample project").click();
    h.run_steps(3);
    assert!(!h.state().ui.windows.start && !h.state().ui.windows.new_project);
    assert!(!h.state().ui.prefs.start_window, "unticked: not shown next time");
    assert_eq!(h.state().session.doc.project.texture_sets.len(), 2, "the sample stays open");
}
