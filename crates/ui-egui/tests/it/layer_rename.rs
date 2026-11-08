//! Renaming a layer in the stack: Enter, a click elsewhere or Escape end the editor, and the
//! shortcuts (Undo) work again afterwards.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> (Harness<'static, TextureCraftApp>, u64) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let layer = h.state().session.selected_layer().unwrap().0;
    (h, layer)
}

fn name(h: &Harness<'_, TextureCraftApp>, layer: u64) -> String {
    h.state().session.doc.project.layers.values().find(|l| l.id.0 == layer).unwrap().name.clone()
}

/// Open the editor on `layer`, select its text and type `text`.
fn start(h: &mut Harness<'_, TextureCraftApp>, layer: u64, text: &str) {
    h.state_mut().ui.rename = Some((layer, name(h, layer)));
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(3);
}

#[test]
fn enter_renames_and_undo_works_afterwards() {
    let (mut h, layer) = harness();
    let before = name(&h, layer);
    start(&mut h, layer, "Rust streaks");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.rename.is_none(), "Enter closes the editor");
    assert_eq!(name(&h, layer), "Rust streaks");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(name(&h, layer), before, "Ctrl+Z undoes the rename");
}

#[test]
fn a_click_elsewhere_renames() {
    let (mut h, layer) = harness();
    start(&mut h, layer, "Clicked away");
    let edit = h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("Clicked away")).rect();
    // Below the editor's row, in the layer stack.
    let away = egui::pos2(edit.center().x, edit.bottom() + 120.0);
    h.drag_at(away);
    h.drop_at(away);
    h.run_steps(3);
    assert!(h.state().ui.rename.is_none(), "a click elsewhere closes the editor");
    assert_eq!(name(&h, layer), "Clicked away");
}

#[test]
fn escape_cancels() {
    let (mut h, layer) = harness();
    let before = name(&h, layer);
    start(&mut h, layer, "Never applied");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.rename.is_none(), "Escape closes the editor");
    assert_eq!(name(&h, layer), before);
}
