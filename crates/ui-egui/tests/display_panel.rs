//! Display Settings picks its tone mapping from a dropdown and leaves the AO intensity to Shader Settings

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// Small texture and bake sizes keep the sample session fast; the panel doesn't depend on them
const TEXTURE_RESOLUTION: u32 = 256;
const BAKE_RESOLUTION: u32 = 64;

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

#[test]
fn display_picks_the_tone_mapping_from_a_dropdown() {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.dock.focus(Panel::Display);
    harness.run_steps(STARTUP_FRAMES);

    let tone_mapping = harness.get_all_by_role(Role::ComboBox).find(|combo| combo.accesskit_node().value().as_deref() == Some("ACES filmic"));
    assert!(tone_mapping.is_some(), "the tone mapping is a dropdown showing ACES filmic");
    assert!(harness.query_by_label("Ambient occlusion intensity").is_none(), "the AO intensity is in Shader Settings");
}
