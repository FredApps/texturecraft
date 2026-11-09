//! The divider between the 3D and 2D views in the split layout resizes them and keeps the pointer from the views

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::UiState;
use texturecraft_ui_egui::ui_state_store::UiStateStore;

/// Narrowest a split view may get, as in `viewport.rs`
const MIN_VIEW_WIDTH: f32 = 16.0;

/// A press clear of the divider's 8 point grab zone and egui's 5 point interact radius
const BESIDE_DIVIDER: f32 = 16.0;

/// App storage held in memory, saved the way the app saves its state between runs
#[derive(Default)]
struct MemoryStorage(std::collections::HashMap<String, String>);

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

/// The sample project in the default split layout at 1600 x 960
fn split_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness
}

/// Drag with `button` from `from` to `to` in a few moves
fn drag(harness: &mut Harness<'_, TextureCraftApp>, button: PointerButton, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run_steps(1);
    }

    harness.event(Event::PointerButton { pos: to, button, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
}

/// The divider's x position: the 3D view ends 1 point left of it
fn divider_x(harness: &Harness<'_, TextureCraftApp>) -> f32 {
    harness.state().viewport_rect.right() + 1.0
}

/// Allocated tiles in the selected layer's base color
fn painted_tiles(harness: &Harness<'_, TextureCraftApp>) -> usize {
    let session = &harness.state().session;
    let layer = session.selected_layer().expect("a selected layer");
    session.doc.pixels.layer(layer, Channel::BaseColor).map_or(0, |image| image.allocated_count())
}

/// The 2D view's texture as drawn: its centre's offset from the view's centre and its half width and height, in pixels at 1 point per pixel
fn uv_texture_on_screen(harness: &Harness<'_, TextureCraftApp>, views_right: f32) -> [f32; 4] {
    let app = harness.state();

    // egui draws the view's frame into its rect rounded to whole pixels
    let width = views_right.round() - (divider_x(harness) + 1.0).round();
    let height = app.viewport_rect.bottom().round() - app.viewport_rect.top().round();
    let texture_size = app.session.doc.project.texture_sets[app.session.ts()].size();
    let [scale_x, scale_y] = texturecraft_render::uv_view_scale(width / height, app.ui.uv_zoom, texture_size);
    let [pan_x, pan_y] = app.ui.uv_pan;
    [pan_x * width * 0.5, pan_y * height * 0.5, scale_x * width * 0.5, scale_y * height * 0.5]
}

/// The 3D camera's yaw and pitch
fn camera_angles(harness: &Harness<'_, TextureCraftApp>) -> (f32, f32) {
    let camera = &harness.state().session.camera;
    (camera.yaw, camera.pitch)
}

#[test]
fn dragging_the_divider_resizes_the_views_within_their_minimum_width() {
    let mut harness = split_app();
    let views_left = harness.state().viewport_rect.left();
    let start_x = divider_x(&harness);
    let views_width = (start_x - views_left) * 2.0;
    let views_right = views_left + views_width;
    let y = harness.state().viewport_rect.center().y;
    assert_eq!(harness.state().ui.split_ratio, 0.5);

    drag(&mut harness, PointerButton::Primary, pos2(start_x, y), pos2(start_x - 200.0, y));
    assert!((divider_x(&harness) - (start_x - 200.0)).abs() < 0.5, "follows the pointer: {}", divider_x(&harness));
    assert!((harness.state().ui.split_ratio - (start_x - 200.0 - views_left) / views_width).abs() < 1e-3);

    let x = divider_x(&harness);
    drag(&mut harness, PointerButton::Primary, pos2(x, y), pos2(views_left + 10.0, y));
    assert!((divider_x(&harness) - views_left - MIN_VIEW_WIDTH).abs() < 0.5, "clamped on the left: {}", divider_x(&harness));

    let x = divider_x(&harness);
    drag(&mut harness, PointerButton::Primary, pos2(x, y), pos2(views_right - 10.0, y));
    assert!((views_right - divider_x(&harness) - MIN_VIEW_WIDTH).abs() < 0.5, "clamped on the right: {}", divider_x(&harness));
}

#[test]
fn pressing_on_the_divider_neither_paints_nor_moves_the_camera() {
    let mut harness = split_app();

    // Pan the texture's centre under the divider so a press there would paint if the 2D view got it
    harness.state_mut().ui.uv_pan[0] = -1.0;
    harness.run_steps(2);
    let x = divider_x(&harness);
    let y = harness.state().viewport_rect.center().y;
    let (tiles, angles) = (painted_tiles(&harness), camera_angles(&harness));

    harness.hover_at(pos2(x, y));
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: pos2(x, y), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.event(Event::PointerButton { pos: pos2(x, y), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
    drag(&mut harness, PointerButton::Primary, pos2(x, y), pos2(x, y + 120.0));
    drag(&mut harness, PointerButton::Secondary, pos2(x, y), pos2(x + 120.0, y));
    assert_eq!(painted_tiles(&harness), tiles, "nothing painted");
    assert_eq!(camera_angles(&harness), angles, "no orbit");
    assert_eq!(divider_x(&harness), x, "a vertical or right-button drag leaves the divider in place");

    // Beside the divider the views get the same input again
    drag(&mut harness, PointerButton::Primary, pos2(x + BESIDE_DIVIDER, y), pos2(x + BESIDE_DIVIDER, y + 40.0));
    assert!(painted_tiles(&harness) > tiles, "the 2D view paints");
    drag(&mut harness, PointerButton::Secondary, pos2(x - BESIDE_DIVIDER, y), pos2(x - BESIDE_DIVIDER - 120.0, y));
    assert_ne!(camera_angles(&harness), angles, "the 3D view orbits");
}

#[test]
fn the_divider_position_survives_a_restart_and_old_saved_states_load_centred() {
    let mut storage = MemoryStorage::default();
    let mut ui_state = UiState::default();
    ui_state.split_ratio = 0.3;
    UiStateStore::default().save(&ui_state, &mut storage);
    let restored = UiStateStore::default().load(Some(&storage)).expect("saved state loads");
    assert_eq!(restored.split_ratio, 0.3);

    // A state saved before the divider existed has no ratio
    let mut saved: serde_json::Value = serde_json::from_str(&storage.0["ui.json"]).unwrap();
    assert!(saved.as_object_mut().unwrap().remove("split_ratio").is_some(), "the saved state names the ratio: {saved}");
    storage.0.insert("ui.json".to_owned(), saved.to_string());
    let restored = UiStateStore::default().load(Some(&storage)).expect("an older saved state loads");
    assert_eq!(restored.split_ratio, 0.5);
}

#[test]
fn dragging_the_divider_keeps_the_uv_texture_size_and_offset_on_screen() {
    // Zoomed in, a 2D sliver needs a zoom far past the one the view starts with
    for zoom in [1.0, 4.0] {
        let mut harness = split_app();
        harness.state_mut().ui.uv_zoom = zoom;
        harness.state_mut().ui.uv_pan = [0.2, 0.1];
        harness.run_steps(2);
        // The split starts centred, so the views end as far right of the divider as they start left of it
        let views_left = harness.state().viewport_rect.left();
        let views_right = views_left + (divider_x(&harness) - views_left) * 2.0;
        let y = harness.state().viewport_rect.center().y;
        let before = uv_texture_on_screen(&harness, views_right);

        let assert_unchanged = |harness: &Harness<'_, TextureCraftApp>| {
            let after = uv_texture_on_screen(harness, views_right);
            for (was, is) in before.iter().zip(after) {
                assert!((was - is).abs() < 0.5, "zoom {zoom}, divider at {}: {before:?} became {after:?}", divider_x(harness));
            }
        };

        for to in [views_left + 300.0, views_right - 150.0, views_right - 10.0, views_left + 500.0] {
            let x = divider_x(&harness);
            drag(&mut harness, PointerButton::Primary, pos2(x, y), pos2(to, y));
            assert_unchanged(&harness);
        }

        // A slow drag past the right end, checked every frame for a nudge at the pixel rounding or the stop
        let from = pos2(divider_x(&harness), y);
        harness.hover_at(from);
        harness.run_steps(1);
        harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.run_steps(1);
        let mut pointer_x = from.x;
        while pointer_x < views_right + 20.0 {
            // Not a whole number of pixels, so the divider lands at every kind of rounding
            pointer_x += 3.3;
            harness.hover_at(pos2(pointer_x, y));
            harness.run_steps(1);
            assert_unchanged(&harness);
        }
        harness.event(Event::PointerButton { pos: pos2(pointer_x, y), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);
        assert_unchanged(&harness);

        // Inside the wheel's 0.1 to 40 range, so the next scroll in the narrow view can't snap the zoom
        let stopped_zoom = harness.state().ui.uv_zoom;
        assert!((0.1..=40.0).contains(&stopped_zoom), "zoom {zoom} stopped at {stopped_zoom}");
    }
}
