//! Shelf hover previews render after a dwell and appear above the asset name

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::materials::{MATERIALS, SmartMaterial};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf_preview::ShelfPreview;

/// The shelf's Materials tab
const MATERIALS_TAB: usize = 0;
/// Popup image size in logical points, independent of display scale
const BIG_PREVIEW_SIZE: f32 = 256.0;
/// Small time steps distinguish preview scheduling from popup visibility
const STEP_SECONDS: f32 = 0.001;
/// Simulated delays in seconds for the popup and preview request
const POPUP_DELAY: f32 = 0.5;
const REST: f32 = 0.1;
/// Real time the worker gets to render
const WORKER_TIMEOUT: Duration = Duration::from_secs(60);

/// Open the Materials tab and find two labels unique to shelf tiles
fn materials_shelf(step_seconds: f32, pixels_per_point: f32) -> (Harness<'static, TextureCraftApp>, Vec<&'static SmartMaterial>) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_step_dt(step_seconds)
        .with_pixels_per_point(pixels_per_point)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.shelf_tab = MATERIALS_TAB;
    h.run_steps(4);

    let unique: Vec<_> = MATERIALS.iter().filter(|m| !m.smart && h.query_all_by_label(m.label).count() == 1).take(2).collect();
    assert_eq!(unique.len(), 2, "two materials with unique names");
    (h, unique)
}

/// Largest drawn texture in pixels, excluding fonts and images below `min_width` points
fn largest_drawn_texture(h: &Harness<'_, TextureCraftApp>, min_width: f32) -> Option<[usize; 2]> {
    let textures = h.ctx.tex_manager();
    let textures = textures.read();
    h.output()
        .shapes
        .iter()
        .filter(|clipped| clipped.shape.texture_id() != egui::TextureId::default() && clipped.shape.visual_bounding_rect().width() >= min_width)
        .filter_map(|clipped| textures.meta(clipped.shape.texture_id()).map(|meta| meta.size))
        .max()
}

/// Check the cache without submitting a preview request
fn big_preview_ready(h: &mut Harness<'static, TextureCraftApp>, material: &SmartMaterial) -> bool {
    let ctx = h.ctx.clone();
    h.state_mut().thumbs.rendered_big(&ctx, &ShelfPreview::Material(material.id), BIG_PREVIEW_SIZE as usize).is_some()
}

#[test]
fn hovering_a_shelf_tile_shows_its_big_preview() {
    let (mut h, materials) = materials_shelf(1.0 / 4.0, 1.0);
    let material = materials[0];
    let tile = h.get_by_label(material.label).rect();
    assert!(largest_drawn_texture(&h, BIG_PREVIEW_SIZE).is_none(), "nothing big before the hover");
    h.get_by_label(material.label).hover();

    let big_px = BIG_PREVIEW_SIZE as usize;
    let start = Instant::now();
    while largest_drawn_texture(&h, BIG_PREVIEW_SIZE) != Some([big_px, big_px]) && start.elapsed() < WORKER_TIMEOUT {
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(largest_drawn_texture(&h, BIG_PREVIEW_SIZE), Some([big_px, big_px]), "the popup draws the big preview");
    let popup_name = h.get_all_by_label(material.label).map(|n| n.rect()).find(|r| r.min.y > tile.max.y).expect("the popup shows the name");
    assert!(popup_name.min.y - tile.max.y >= BIG_PREVIEW_SIZE, "{popup_name:?} under {tile:?}");
}

#[test]
fn the_big_preview_renders_after_a_rest_but_not_during_a_sweep() {
    let (mut h, materials) = materials_shelf(STEP_SECONDS, 1.0);
    let (first, second) = (materials[0], materials[1]);
    let (from, to) = (h.get_by_label(first.label).rect().center(), h.get_by_label(second.label).rect().center());

    // Moving every frame must never satisfy the dwell delay
    let sweep_frames = 40;
    for frame in 0..=sweep_frames {
        h.hover_at(from.lerp(to, frame as f32 / sweep_frames as f32));
        h.run_steps(1);
    }
    h.hover_at(egui::pos2(900.0, 500.0));
    h.run_steps(1);
    std::thread::sleep(Duration::from_secs(1));
    h.run_steps(1);
    assert!(!big_preview_ready(&mut h, first) && !big_preview_ready(&mut h, second), "a sweep renders nothing");

    // A stationary pointer should start rendering before the popup opens
    h.hover_at(from);
    h.run_steps((REST * 1.2 / STEP_SECONDS) as usize);
    let start = Instant::now();
    let headroom_frames = ((POPUP_DELAY - REST * 1.2) / STEP_SECONDS) as usize - 1;
    for _ in 0..headroom_frames {
        if big_preview_ready(&mut h, first) || start.elapsed() > WORKER_TIMEOUT {
            break;
        }
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(big_preview_ready(&mut h, first), "the preview rendered during the popup's delay");
    assert_eq!(h.query_all_by_label(first.label).count(), 1, "the popup is not open yet");
}

#[test]
fn scrolling_with_a_stationary_pointer_defers_the_big_preview() {
    let (mut harness, materials) = materials_shelf(STEP_SECONDS, 1.0);
    let material = materials[0];
    harness.get_by_label(material.label).hover();

    // Scroll toward the top boundary to keep the same tile beneath the pointer
    for _ in 0..(REST * 2.0 / STEP_SECONDS) as usize {
        harness.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 10.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(1);
    }
    assert!(harness.ctx.input(|input| input.pointer.time_since_last_movement()) >= REST, "the pointer stayed still beyond the dwell delay");
    assert_eq!(harness.query_all_by_label(material.label).count(), 1, "scrolling suppresses the popup");

    harness.hover_at(egui::pos2(900.0, 500.0));
    harness.run_steps(1);
    std::thread::sleep(Duration::from_secs(1));
    harness.run_steps(1);
    assert!(!big_preview_ready(&mut harness, material), "scrolling must not submit a hover preview");

    harness.get_by_label(material.label).hover();
    let started = Instant::now();
    while !big_preview_ready(&mut harness, material) && started.elapsed() < WORKER_TIMEOUT {
        harness.run_steps(1);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(big_preview_ready(&mut harness, material), "the preview renders after scrolling and pointer movement stop");
}

#[test]
fn high_dpi_smart_mask_hover_stays_within_the_renderer_limit() {
    use texturecraft_engine::smart_masks::SMART_MASKS;

    check_high_dpi_orb_hover(2, SMART_MASKS[0].label);
}

#[test]
fn high_dpi_smart_material_hover_stays_within_the_renderer_limit() {
    check_high_dpi_orb_hover(1, "Steel Painted Worn");
}

fn check_high_dpi_orb_hover(shelf_tab: usize, label: &str) {
    use texturecraft_engine::orb_preview::MAX_PREVIEW_SIZE_PX;

    let (mut harness, _) = materials_shelf(1.0 / 4.0, 3.0);
    harness.state_mut().ui.shelf_tab = shelf_tab;
    harness.run_steps(4);
    assert_eq!(harness.ctx.pixels_per_point(), 3.0, "the test uses the native 300% display scale");
    harness.get_by_role_and_label(egui::Role::Button, label).hover();

    let expected = Some([MAX_PREVIEW_SIZE_PX; 2]);
    let started = Instant::now();
    while largest_drawn_texture(&harness, BIG_PREVIEW_SIZE) != expected && started.elapsed() < WORKER_TIMEOUT {
        harness.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(largest_drawn_texture(&harness, BIG_PREVIEW_SIZE), expected, "the popup uses a bounded big preview at 300% scale");
}
