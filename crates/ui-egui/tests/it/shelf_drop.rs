//! A shelf material dropped on the 3D view goes to the texture set under the pointer

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf::{Asset, ShelfDrag};

/// Distance between the view points searched for a set or the background
const SEARCH_STEP: f32 = 8.0;

/// The sample project (Ball = set 0, Base = set 1) with the Ball selected, at 1600 x 960
fn sample_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    assert_eq!(harness.state().session.ts(), 0);
    harness
}

/// The first point of the 3D view showing texture set `set`, or the background when `None`
fn point_over(harness: &Harness<'_, TextureCraftApp>, set: Option<usize>) -> Pos2 {
    let app = harness.state();
    let rect = app.viewport_rect;
    let steps = |length: f32| (1..(length / SEARCH_STEP) as usize).map(|step| step as f32 * SEARCH_STEP);

    steps(rect.height())
        .flat_map(|y| steps(rect.width()).map(move |x| (x, y)))
        .find(|&(x, y)| app.session.texture_set_at(Vec2::new(x, y), app.session.viewport) == set)
        .map(|(x, y)| rect.min + egui::vec2(x, y))
        .unwrap()
}

/// Drop a built-in shelf material at `position`, as a drag from the shelf ends there
fn drop_material(harness: &mut Harness<'_, TextureCraftApp>, position: Pos2) {
    egui::DragAndDrop::set_payload(&harness.ctx, ShelfDrag { asset: Asset::Material("iron_rusted"), label: "Iron".into() });
    harness.hover_at(position);
    harness.run_steps(2);

    harness.event(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
}

/// Top-level layer count of each texture set
fn stack_sizes(harness: &Harness<'_, TextureCraftApp>) -> Vec<usize> {
    harness.state().session.doc.project.texture_sets.iter().map(|set| set.stack.len()).collect()
}

#[test]
fn a_material_dropped_on_another_set_goes_to_that_set_and_selects_it() {
    let mut harness = sample_app();
    let before = stack_sizes(&harness);

    let base = point_over(&harness, Some(1));
    drop_material(&mut harness, base);

    assert_eq!(harness.state().session.ts(), 1, "the set under the pointer is selected");
    assert_eq!(stack_sizes(&harness), vec![before[0], before[1] + 1], "only the set under the pointer gets the material");
}

#[test]
fn a_material_dropped_on_the_background_changes_nothing() {
    let mut harness = sample_app();
    let (stacks, history) = (stack_sizes(&harness), harness.state().session.history_labels().len());

    let background = point_over(&harness, None);
    drop_material(&mut harness, background);

    assert_eq!(harness.state().session.ts(), 0);
    assert_eq!(stack_sizes(&harness), stacks);
    assert_eq!(harness.state().session.history_labels().len(), history);
}
