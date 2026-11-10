//! The History and Log lists fill their panel instead of stopping at a fixed height

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::{Dock, Panel};

/// The list heights the panels used to stop at
const OLD_HISTORY_HEIGHT: f32 = 320.0;
const OLD_LOG_HEIGHT: f32 = 300.0;

/// Leaves the left column (where the panel is docked) between 700 and 800 points tall
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 860.0);

/// Right of the left column (11.5% of the window, about 184 points) and left of every other panel's scroll bar
const LEFT_COLUMN_MAX_X: f32 = 300.0;

/// Enough rows to overflow the tallest panel
const ROWS: usize = 80;

/// Small texture and bake sizes keep the sample session fast; the panels don't depend on them
const TEXTURE_RESOLUTION: u32 = 256;
const BAKE_RESOLUTION: u32 = 64;

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

/// Dock `panel` in the full-height left column, where the Assets panel is by default
fn dock_in_left_column(harness: &mut Harness<'_, TextureCraftApp>, panel: &str) {
    // Each panel name appears once in the saved dock: in the layout or in the hidden list
    let saved = serde_json::to_string(&harness.state().ui.dock).unwrap();
    let placeholder = "\"swapped-out\"";
    let swapped = saved.replace("\"Assets\"", placeholder).replace(&format!("\"{panel}\""), "\"Assets\"").replace(placeholder, &format!("\"{panel}\""));
    harness.state_mut().ui.dock = Dock::from_value(serde_json::from_str(&swapped).unwrap());
}

/// The height of the list's scroll bar, which runs the visible height of the list
fn list_height(harness: &Harness<'_, TextureCraftApp>) -> f32 {
    let bar = harness.get_all_by_role(Role::ScrollBar).find(|bar| bar.rect().max.x < LEFT_COLUMN_MAX_X).expect("a scroll bar in the left column");
    bar.rect().height()
}

#[test]
fn the_history_list_fills_a_tall_panel() {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let layer = session.selected_layer().unwrap();
    for step in 0..ROWS {
        texturecraft_engine::execute(&mut session, "layer.rename", &json!({"layer": layer.0, "name": format!("Step {step}")})).unwrap();
    }

    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    dock_in_left_column(&mut harness, "History");
    harness.run_steps(STARTUP_FRAMES);

    assert!(harness.state().ui.dock.is_visible(Panel::History));
    let height = list_height(&harness);
    assert!(height > OLD_HISTORY_HEIGHT, "the history list is {height} points tall");
}

#[test]
fn the_log_list_fills_a_tall_panel() {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().session.log.extend((0..ROWS).map(|line| format!("Log line {line}")));
    dock_in_left_column(&mut harness, "Log");
    harness.run_steps(STARTUP_FRAMES);

    assert!(harness.state().ui.dock.is_visible(Panel::Log));
    let height = list_height(&harness);
    assert!(height > OLD_LOG_HEIGHT, "the log list is {height} points tall");
}
