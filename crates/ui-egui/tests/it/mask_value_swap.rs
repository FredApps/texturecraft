//! X inverts the mask value while a mask is the paint target; Ctrl+X still cuts the layer

use egui_kittest::Harness;
use texturecraft_engine::StackTarget;
use texturecraft_ui_egui::TextureCraftApp;

/// The sample session with a white mask on its selected layer, painting that mask
fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    texturecraft_engine::execute(&mut session, "mask.add.white", &serde_json::Value::Null).unwrap();
    session.target = StackTarget::Mask;
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);

    harness
}

fn mask_value(harness: &Harness<'_, TextureCraftApp>) -> f32 {
    harness.state().session.tool.mask_value
}

#[test]
fn x_inverts_the_mask_value_and_back() {
    let mut harness = harness();
    harness.state_mut().session.tool.mask_value = 0.25;

    harness.key_press(egui::Key::X);
    harness.run_steps(3);

    assert_eq!(mask_value(&harness), 0.75);

    harness.key_press(egui::Key::X);
    harness.run_steps(3);

    assert_eq!(mask_value(&harness), 0.25);
}

#[test]
fn x_does_nothing_while_painting_layer_content() {
    let mut harness = harness();
    harness.state_mut().session.target = StackTarget::Content;
    harness.state_mut().session.tool.mask_value = 0.25;

    harness.key_press(egui::Key::X);
    harness.run_steps(3);

    assert_eq!(mask_value(&harness), 0.25);
}

#[test]
fn ctrl_x_still_cuts_the_layer_while_a_mask_is_the_target() {
    let mut harness = harness();
    let layer = harness.state().session.selected_layer().unwrap();

    harness.event(egui::Event::Cut);
    harness.run_steps(3);

    assert!(!harness.state().session.doc.project.layers.contains_key(&layer), "Ctrl+X removes the layer");
}

#[test]
fn ctrl_x_is_not_x() {
    let mut harness = harness();

    // With cut unbound, nothing else may answer Ctrl+X
    harness.state_mut().ui.keymap.insert("layer.cut".into(), String::new());
    harness.state_mut().session.tool.mask_value = 0.25;

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::X);
    harness.run_steps(3);

    assert_eq!(mask_value(&harness), 0.25);
}
