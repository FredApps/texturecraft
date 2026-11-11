//! Shader Settings shows each of its settings once: the height strength lives in the texture set settings

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// Small texture and bake sizes keep the sample session fast; the panel doesn't depend on them
const TEXTURE_RESOLUTION: u32 = 256;
const BAKE_RESOLUTION: u32 = 64;

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

#[test]
fn shader_settings_leave_the_height_strength_to_the_texture_set_settings() {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.dock.focus(Panel::Shader);
    harness.run_steps(STARTUP_FRAMES);

    assert!(harness.query_by_label("AO intensity").is_some(), "the Shader Settings panel is shown");
    // Texture Set Settings, the slider's other home, sits behind Layers in the default layout, so any match is in Shader Settings
    assert!(!harness.state().ui.dock.is_visible(Panel::TextureSetSettings), "Texture Set Settings is behind Layers");
    assert!(harness.query_by_label("Height to normal strength").is_none());
}
