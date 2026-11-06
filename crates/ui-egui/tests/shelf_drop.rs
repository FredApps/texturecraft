//! A shelf material or image dropped on the 3D view goes to the texture set under the pointer; an image first asks what it becomes

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf::{Asset, ShelfDrag};

/// Distance between the view points searched for a set or the background
const SEARCH_STEP: f32 = 8.0;

/// How far from the drop point the drop menu's items can be
const MENU_REACH: f32 = 60.0;

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
    drop_asset(harness, Asset::Material("iron_rusted"), position);
}

/// Drop shelf `asset` at `position`, as a drag from the shelf ends there
fn drop_asset(harness: &mut Harness<'_, TextureCraftApp>, asset: Asset, position: Pos2) {
    egui::DragAndDrop::set_payload(&harness.ctx, ShelfDrag { asset, label: "Dragged".into() });
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

/// The sample app in English with a white image dropped on texture set `position_over` (the background when `None`)
fn drop_image(position_over: Option<usize>) -> (Harness<'static, TextureCraftApp>, Pos2) {
    let mut harness = sample_app();
    harness.state_mut().ui.prefs.language = "en".into();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = harness.state_mut().session.import_resource_bytes("logo.png", png).unwrap();
    harness.run_steps(2);

    let position = point_over(&harness, position_over);
    drop_asset(&mut harness, Asset::Texture(name), position);
    // The menu's first frame only measures it
    harness.run_steps(2);
    (harness, position)
}

#[test]
fn an_image_dropped_on_another_set_asks_for_a_channel_then_goes_to_that_set() {
    let (mut harness, position) = drop_image(Some(1));
    let before = stack_sizes(&harness);

    // The menu item nearest the drop point (Properties may show the same name)
    let distance = |node: &egui_kittest::Node<'_>| node.rect().center().distance(position);
    harness.get_all_by_label("Roughness").min_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap().click();
    harness.run_steps(2);

    let app = harness.state();
    assert_eq!(app.session.ts(), 1, "the set under the drop point is selected");
    assert_eq!(stack_sizes(&harness), vec![before[0], before[1] + 1]);
    let layer = app.session.selected_layer().and_then(|id| app.session.doc.project.layer(id).ok()).unwrap();
    assert_eq!(layer.channels.keys().copied().collect::<Vec<_>>(), [Channel::Roughness], "the image fills the picked channel");
}

/// Whether the drop menu is open at `position` (its Mask item near there)
fn menu_open(harness: &Harness<'_, TextureCraftApp>, position: Pos2) -> bool {
    harness.query_all_by_label("Mask").any(|node| node.rect().center().distance(position) < MENU_REACH)
}

#[test]
fn an_image_drop_places_nothing_until_a_pick_and_nothing_on_the_background() {
    let (mut harness, position) = drop_image(Some(1));
    let (stacks, history) = (stack_sizes(&harness), harness.state().session.history_labels().len());
    assert!(menu_open(&harness, position));

    // A click elsewhere closes the menu
    let corner = harness.state().viewport_rect.min + egui::vec2(SEARCH_STEP, SEARCH_STEP);
    harness.event(Event::PointerButton { pos: corner, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.event(Event::PointerButton { pos: corner, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    assert!(!menu_open(&harness, position));
    assert_eq!(stack_sizes(&harness), stacks);
    assert_eq!(harness.state().session.history_labels().len(), history);

    let (harness, position) = drop_image(None);
    assert!(!menu_open(&harness, position), "the background opens no menu");
}
