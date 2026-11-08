//! Shift+click in the 2D view paints a straight line from where the last stroke ended, as in the 3D view

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// The first click, on the sample's texture in the 2D view
const FIRST_CLICK: Pos2 = pos2(700.0, 420.0);

/// The Shift+click, on the texture of both of the sample's texture sets
const SECOND_CLICK: Pos2 = pos2(900.0, 520.0);

/// Start of a drag cancelled with Escape, well away from the line between the two clicks
const CANCELLED_DRAG_FROM: Pos2 = pos2(760.0, 700.0);

/// End of the cancelled drag
const CANCELLED_DRAG_TO: Pos2 = pos2(820.0, 700.0);

/// Alpha a texel needs to count as painted
const PAINTED_ALPHA: f32 = 0.05;

/// How far beside the line the unpainted check looks, in brush radii
const BESIDE_LINE_IN_RADII: f32 = 3.0;

/// The sample project with only the 2D view, at 1600 x 960
fn uv_view_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    harness
}

/// A primary click at `pos`, with Shift held when `shift` is set
fn click(harness: &mut Harness<'_, TextureCraftApp>, pos: Pos2, shift: bool) {
    let modifiers = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(2);
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(1);
}

/// Press at `from`, drag to `to`, press Escape and release: the stroke is cancelled
fn cancelled_drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    harness.hover_at(to);
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    assert!(!harness.state().session.stroke_active(), "Escape cancelled the stroke");
}

/// The selected layer's base color alpha, row by row, with the texture width
fn alpha(harness: &Harness<'_, TextureCraftApp>) -> (Vec<f32>, u32) {
    let session = &harness.state().session;
    let (width, height) = session.doc.project.texture_sets[session.ts()].size();
    let layer = session.selected_layer().expect("a selected layer");
    let Some(image) = session.doc.pixels.layer(layer, Channel::BaseColor) else { return (vec![0.0; (width * height) as usize], width) };
    let texels = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
    (texels.map(|(x, y)| image.get(x, y)[3] as f32 / 65535.0).collect(), width)
}

/// Alpha added between two snapshots
fn added(before: &[f32], after: &[f32]) -> Vec<f32> {
    after.iter().zip(before).map(|(after, before)| (after - before).max(0.0)).collect()
}

/// The alpha-weighted centre of `paint`, in texels
fn centre(paint: &[f32], width: u32) -> Vec2 {
    let (sum, total) = paint.iter().enumerate().fold((Vec2::ZERO, 0.0), |(sum, total), (index, value)| {
        let texel = Vec2::new((index as u32 % width) as f32, (index as u32 / width) as f32);
        (sum + texel * *value, total + value)
    });
    assert!(total > 0.0, "something was painted");
    sum / total
}

/// Whether `paint` holds visible alpha at `texel`
fn painted_at(paint: &[f32], width: u32, texel: Vec2) -> bool {
    let index = texel.y.round() as usize * width as usize + texel.x.round() as usize;
    paint[index] > PAINTED_ALPHA
}

/// The brush radius in texels: a 2D view stamp's radius is twice the brush size in UV units
fn brush_radius_in_texels(harness: &Harness<'_, TextureCraftApp>, width: u32) -> f32 {
    harness.state().session.tool.brush.size * 2.0 * width as f32
}

/// Click, run `between` (zoom or pan), Shift+click; assert one straight stroke joins both dabs
fn assert_shift_click_draws_a_line(between: impl FnOnce(&mut Harness<'_, TextureCraftApp>)) {
    let mut harness = uv_view_app();
    let (blank, width) = alpha(&harness);
    click(&mut harness, FIRST_CLICK, false);
    let (first_dab, _) = alpha(&harness);
    let start = centre(&added(&blank, &first_dab), width);

    between(&mut harness);
    harness.run_steps(2);
    let steps_before = harness.state().session.history_labels().len();
    click(&mut harness, SECOND_CLICK, true);
    assert_eq!(harness.state().session.history_labels().len(), steps_before + 1, "the line is one undo step");
    let (with_line, _) = alpha(&harness);
    let line = added(&first_dab, &with_line);

    // A plain click at the same point finds where the line should end
    harness.state_mut().session.undo().unwrap();
    click(&mut harness, SECOND_CLICK, false);
    let (with_dab, _) = alpha(&harness);
    let end = centre(&added(&first_dab, &with_dab), width);

    let middle = (start + end) * 0.5;
    let across = (end - start).perp().normalize() * brush_radius_in_texels(&harness, width) * BESIDE_LINE_IN_RADII;
    assert!(painted_at(&line, width, middle), "the line passes halfway between {start} and {end}");
    assert!(!painted_at(&line, width, middle + across), "nothing is painted beside the line");
}

#[test]
fn shift_click_paints_a_straight_line_from_the_last_stroke() {
    assert_shift_click_draws_a_line(|_| {});
}

#[test]
fn the_line_joins_the_same_texture_points_after_zoom_and_pan() {
    assert_shift_click_draws_a_line(|harness| {
        let ui_state = &mut harness.state_mut().ui;
        ui_state.uv_zoom *= 1.5;
        ui_state.uv_pan = [0.2, -0.1];
    });
}

/// Shift+click at the second point; assert it painted the same dot a plain click there paints
fn assert_shift_click_paints_only_a_dot(harness: &mut Harness<'_, TextureCraftApp>) {
    let (before, _) = alpha(harness);
    click(harness, SECOND_CLICK, true);
    let (after_shift_click, _) = alpha(harness);
    let shift_click_alpha: f32 = added(&before, &after_shift_click).iter().sum();

    harness.state_mut().session.undo().unwrap();
    click(harness, SECOND_CLICK, false);
    let (after_click, _) = alpha(harness);
    let click_alpha: f32 = added(&before, &after_click).iter().sum();

    assert!(click_alpha > 1.0, "the plain click painted: {click_alpha}");
    assert!((shift_click_alpha - click_alpha).abs() < click_alpha * 0.01, "Shift+click paints only a dot: {shift_click_alpha} vs {click_alpha}");
}

#[test]
fn shift_click_without_a_previous_stroke_paints_a_dot() {
    assert_shift_click_paints_only_a_dot(&mut uv_view_app());
}

#[test]
fn a_line_does_not_start_from_a_point_on_another_texture_set() {
    let mut harness = uv_view_app();
    click(&mut harness, FIRST_CLICK, false);

    harness.state_mut().run("textureSet.select", json!({"ts": 1}));
    harness.run_steps(2);

    assert_shift_click_paints_only_a_dot(&mut harness);
}

#[test]
fn a_stroke_cancelled_with_escape_leaves_the_line_start_where_it_was() {
    assert_shift_click_draws_a_line(|harness| cancelled_drag(harness, CANCELLED_DRAG_FROM, CANCELLED_DRAG_TO));
}

#[test]
fn a_new_project_does_not_reuse_the_previous_projects_line_start() {
    let mut harness = uv_view_app();
    click(&mut harness, FIRST_CLICK, false);
    harness.state_mut().run("project.new", json!({"mesh": "sphere", "resolution": 256})).unwrap();
    harness.run_steps(2);

    assert_shift_click_paints_only_a_dot(&mut harness);
}

#[test]
fn a_project_opened_through_control_does_not_reuse_the_line_start() {
    let mut harness = uv_view_app();
    click(&mut harness, FIRST_CLICK, false);
    let context = harness.ctx.clone();
    let result = texturecraft_ui_egui::control::handle(
        harness.state_mut(),
        &context,
        "command.run",
        &json!({"id": "project.new", "params": {"mesh": "sphere", "resolution": 256}}),
    );
    assert_eq!(result["ok"], true);
    harness.run_steps(2);

    assert_shift_click_paints_only_a_dot(&mut harness);
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn shift_line_snapshots() {
    let output_dir = std::path::PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS to the output directory"));
    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|context| {
        let adapter = &context.wgpu_render_state.as_ref().expect("wgpu render state").adapter;
        eprintln!("Shift-line snapshot adapter: {:?}", adapter.get_info());
        TextureCraftApp::new(context, session)
    });
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    click(&mut harness, FIRST_CLICK, false);

    let save = |harness: &mut Harness<'_, TextureCraftApp>, name: &str| {
        harness.run_steps(4);
        let path = output_dir.join(format!("{name}.png"));
        harness.render().unwrap().save(&path).unwrap();
        eprintln!("wrote {}", path.display());
    };

    harness.event(Event::ModifiersChanged(Modifiers::SHIFT));
    harness.hover_at(SECOND_CLICK);
    save(&mut harness, "shift-line-preview");
    click(&mut harness, SECOND_CLICK, true);
    save(&mut harness, "shift-line-painted");

    harness.state_mut().ui.uv_zoom = 1.5;
    harness.state_mut().ui.uv_pan = [0.2, -0.1];
    harness.event(Event::ModifiersChanged(Modifiers::SHIFT));
    harness.hover_at(FIRST_CLICK);
    save(&mut harness, "shift-line-zoom-pan-preview");

    harness.state_mut().run("project.new", json!({"mesh": "sphere", "resolution": 256})).unwrap();
    save(&mut harness, "shift-line-new-project");
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    assert_shift_click_paints_only_a_dot(&mut harness);
}
