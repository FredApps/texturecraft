//! A fill's projection settings fold into a "UV transformations" section under the Projection combo

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;

/// Tall enough that Properties shows the whole fill without scrolling
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 1400.0);

/// Small texture and bake sizes keep the sample session fast; the panel doesn't depend on them
const TEXTURE_RESOLUTION: u32 = 256;
const BAKE_RESOLUTION: u32 = 64;

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

/// Frames for a click or selection change to reach the panel
const SETTLE_FRAMES: usize = 3;

/// The app with a new fill layer (projected as `projection`) selected in Properties, and its layer id
fn app_with_fill(projection: &str) -> (Harness<'static, TextureCraftApp>, u64) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let added = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let layer = added["layer"].as_u64().unwrap();
    texturecraft_engine::execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "projection": projection})).unwrap();

    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, layer)
}

fn toggle_section(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.get_by_label("UV transformations").click();
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_uv_transformations_start_open_and_fold_away() {
    let (mut harness, _) = app_with_fill("uv");
    assert!(harness.query_by_label("Rotation").is_some());

    toggle_section(&mut harness);
    assert!(harness.query_by_label("Rotation").is_none());
    assert!(harness.query_by_label("Scale / tiling").is_none());
    assert!(harness.query_by_label("Projection").is_some());

    toggle_section(&mut harness);
    assert!(harness.query_by_label("Rotation").is_some());
}

#[test]
fn hardness_and_axis_stay_outside_the_section() {
    let (mut harness, _) = app_with_fill("triplanar");
    toggle_section(&mut harness);
    assert!(harness.query_by_label("Offset X").is_none());
    assert!(harness.query_by_label("Hardness").is_some());

    let (mut harness, _) = app_with_fill("planar");
    toggle_section(&mut harness);
    assert!(harness.query_by_label("Offset X").is_none());
    assert!(harness.query_by_label("Axis").is_some());

    let (mut harness, _) = app_with_fill("cylindrical");
    toggle_section(&mut harness);
    assert!(harness.query_by_label("Scale / tiling").is_none());
    assert!(harness.query_by_label("Axis").is_some());
}

#[test]
fn each_fill_keeps_its_own_open_state() {
    let (mut harness, first) = app_with_fill("uv");
    toggle_section(&mut harness);

    let added = texturecraft_engine::execute(&mut harness.state_mut().session, "layer.add.fill", &json!({})).unwrap();
    let second = added["layer"].as_u64().unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": second})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.query_by_label("Rotation").is_some(), "a new fill starts open");

    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": first})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.query_by_label("Rotation").is_none(), "the first fill stays closed");
}

#[test]
fn a_fill_effect_keeps_a_state_apart_from_its_layer() {
    let (mut harness, layer) = app_with_fill("uv");
    toggle_section(&mut harness);

    let session = &mut harness.state_mut().session;
    let added = texturecraft_engine::execute(session, "effect.add.fill", &json!({"layer": layer, "target": "content"})).unwrap();
    let effect = added["effect"].as_u64().unwrap();
    texturecraft_engine::execute(session, "layer.select", &json!({"layer": layer, "target": effect})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.query_by_label("Rotation").is_some(), "the effect's section starts open");

    toggle_section(&mut harness);
    assert!(harness.query_by_label("Rotation").is_none());
}
