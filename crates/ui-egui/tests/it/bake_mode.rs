use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::{BakeResolution, BakeSettings, BakedMap, CageType, CurvatureMethod, HeightNormalization};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

/// A bake-mode app with only `map` enabled and Mesh Map Settings in front
fn harness(map: Option<BakedMap>) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.doc.project.texture_sets[0].bake = Default::default();
    session.doc.project.texture_sets[0].bake.common.resolution = BakeResolution::R64;
    session.doc.project.texture_sets[0].bake.maps.select(map);
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::MeshMapSettings);
    harness.run_steps(3);
    harness
}

/// The maps the first texture set will bake
fn enabled_maps(harness: &Harness<'_, TextureCraftApp>) -> Vec<BakedMap> {
    harness.state().session.doc.project.texture_sets[0].bake.maps.enabled().collect()
}

/// Click a map's row in Mesh Map Bakers, which shows its settings
fn select_map(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    harness.get_by_role_and_label(Role::Button, name).click();
    harness.run_steps(3);
}

/// Click a checkbox in Mesh Map Bakers: a map's row or Check All
fn click_checkbox(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    harness.get_by_role_and_label(Role::CheckBox, name).click();
    harness.run_steps(3);
}

/// Scroll the widget labelled `label` into view and click it
fn click(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    harness.run_steps(3);
}

fn number_field<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &str) -> egui_kittest::Node<'a> {
    let label_row = harness.get_by_label(label).rect().center().y;
    harness.get_all_by_role(Role::TextInput).find(|node| (node.rect().center().y - label_row).abs() < 4.0).unwrap()
}

#[test]
fn mesh_map_edits_update_all_sets_without_copying_unedited_fields() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    assert_eq!(session.set_count(), 2);
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.common.resolution_height = Some(BakeResolution::R256);
    second.maps.ao.is_enabled = false;
    second.maps.ao.parameters.rays.rays = 16;
    second.maps.ao.parameters.ground_plane.offset = 3.0;
    second.maps.height.parameters.scaling_divisor = 2.0;
    let before = session.doc.project.clone();
    let history_steps = session.history_labels().len();

    click_checkbox(&mut harness, "Ground Plane");

    let mut expected = before.clone();
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.maps.ao.parameters.ground_plane.enabled = true;
    }
    let session = &mut harness.state_mut().session;
    assert_eq!(session.doc.project, expected);
    assert_eq!(session.history_labels().len(), history_steps + 1);
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
    session.redo().unwrap();
    assert_eq!(session.doc.project, expected);
}

#[test]
fn common_settings_edits_update_all_sets_and_keep_their_map_selection() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.maps.select([BakedMap::Height]);
    let before = session.doc.project.clone();

    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    click_checkbox(&mut harness, "Apply Diffusion");

    let mut expected = before.clone();
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.common.apply_diffusion = false;
    }
    assert_eq!(harness.state().session.doc.project, expected);

    harness.get_all_by_value("64").next().unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "256");
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.common.resolution = BakeResolution::R256;
    }
    assert_eq!(harness.state().session.doc.project, expected);

    click_checkbox(&mut harness, "Use Low Poly Mesh as High Poly Mesh");
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.high_poly.use_low_as_high = true;
    }
    assert_eq!(harness.state().session.doc.project, expected);
}

#[test]
fn secondary_rays_edit_from_another_set_updates_all_sets() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    session.doc.project.texture_sets[1].bake = BakeSettings::default();
    session.doc.project.texture_sets[1].bake.maps.ao.parameters.rays.rays = 16;
    texturecraft_engine::execute(session, "textureSet.select", &serde_json::json!({"ts": 1})).unwrap();
    harness.run_steps(3);
    select_map(&mut harness, "Ambient Occlusion");
    let history_steps = harness.state().session.history_labels().len();

    number_field(&harness, "Secondary Rays").focus();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(1);
    number_field(&harness, "Secondary Rays").type_text("128");
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);

    let session = &harness.state().session;
    assert_eq!(session.ts(), 1);
    for texture_set in &session.doc.project.texture_sets {
        assert_eq!(texture_set.bake.maps.ao.parameters.rays.rays, 128);
    }
    assert_eq!(session.history_labels().len(), history_steps + 1);
}

#[test]
fn map_selection_stays_per_set() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let second = harness.state().session.doc.project.texture_sets[1].bake.clone();

    click_checkbox(&mut harness, "Ambient Occlusion");
    assert!(enabled_maps(&harness).is_empty());
    assert_eq!(harness.state().session.doc.project.texture_sets[1].bake, second);

    click_checkbox(&mut harness, "Check All");
    assert_eq!(enabled_maps(&harness), BakedMap::ALL);
    assert_eq!(harness.state().session.doc.project.texture_sets[1].bake, second);
}

#[test]
fn bake_texture_set_visibility_is_independent_of_bake_selection() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let before: Vec<_> = harness.state().session.doc.project.texture_sets.iter().map(|texture_set| texture_set.bake.clone()).collect();
    let active_set = harness.state().session.ts();

    harness.get_all_by_label("Show or hide the texture set").nth(1).unwrap().click();
    harness.run_steps(3);

    let session = &harness.state().session;
    assert!(session.doc.project.texture_sets[0].visible);
    assert!(!session.doc.project.texture_sets[1].visible);
    assert_eq!(session.ts(), active_set);
    for (texture_set, settings) in session.doc.project.texture_sets.iter().zip(&before) {
        assert_eq!(&texture_set.bake, settings);
    }

    harness.get_all_by_label("Show or hide the texture set").nth(1).unwrap().click();
    harness.run_steps(3);
    assert!(harness.state().session.doc.project.texture_sets[1].visible);
}

#[test]
fn texture_set_resolution_controls_are_independent_from_bake_output() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let before = harness.state().session.doc.project.clone();
    let active_set = harness.state().session.ts();
    harness.get_all_by_value("256 × 256").nth(1).unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "128 × 128");
    wait_for_resolution(&mut harness, 1, 128);

    let session = &harness.state().session;
    assert_eq!(session.ts(), active_set);
    assert_eq!(session.doc.project.texture_sets[0], before.texture_sets[0]);
    assert_eq!(session.doc.project.texture_sets[1].bake, before.texture_sets[1].bake);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(harness.state().session.doc.project, before);
}

fn wait_for_resolution(harness: &mut Harness<'_, TextureCraftApp>, texture_set: usize, resolution: u32) {
    for _ in 0..100 {
        harness.run_steps(1);
        if harness.state().session.doc.project.texture_sets[texture_set].resolution == resolution {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("texture set resize did not finish");
}

#[test]
fn tile_resolution_overrides_can_be_changed_and_removed_in_bake_mode() {
    let session = texturecraft_engine::Session::new_project(
        "Tile resolution controls",
        texturecraft_engine::geom::primitives::udim_plane(),
        "builtin:udim_plane",
        None,
        256,
    )
    .unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    let before = harness.state().session.doc.project.clone();

    assert_eq!(harness.get_all_by_value("256 × 256").count(), 1);
    let resolution_position = harness.get_by_label("UV tile 1002 resolution").rect().center();
    harness.hover_at(resolution_position);
    harness.run_steps(3);
    assert_eq!(harness.get_all_by_value("256 × 256").count(), 2);
    harness.hover_at(egui::pos2(900.0, 500.0));
    harness.run_steps(3);
    assert_eq!(harness.get_all_by_value("256 × 256").count(), 1);
    harness.get_by_label("UV tile 1002 resolution").click();
    harness.run_steps(3);
    click(&mut harness, "128 × 128");
    harness.get_by_value("128 × 128*");
    let texture_set = &harness.state().session.doc.project.texture_sets[0];
    assert_eq!(texture_set.resolution, 256);
    assert_eq!(texture_set.tile_resolution(1001), 256);
    assert_eq!(texture_set.tile_resolution(1002), 128);
    assert_eq!(texture_set.bake, before.texture_sets[0].bake);

    harness.get_by_value("128 × 128*").click();
    harness.run_steps(3);
    click(&mut harness, "Remove resolution override");
    assert_eq!(harness.state().session.doc.project, before);
    assert!(harness.query_by_value("128 × 128*").is_none());
    assert_eq!(harness.get_all_by_value("256 × 256").count(), 1);
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);
    harness.get_by_value("128 × 128*");

    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_all_by_value("2048").next().unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "64");
    let texture_set = &harness.state().session.doc.project.texture_sets[0];
    assert_eq!(texture_set.resolution, 256);
    assert_eq!(texture_set.tile_resolution(1002), 128);
    assert_eq!(texture_set.bake.common.resolution, BakeResolution::R64);
}

#[test]
fn bake_texture_sets_list_uv_tiles_and_their_resolutions() {
    let mut session =
        texturecraft_engine::Session::new_project("UV tile list", texturecraft_engine::geom::primitives::udim_plane(), "builtin:udim_plane", None, 256)
            .unwrap();
    texturecraft_engine::execute(&mut session, "textureSet.tile.resolution", &serde_json::json!({"tile": 1002, "resolution": 128})).unwrap();
    let before = session.doc.project.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);

    harness.get_by_label("1001");
    harness.get_by_value("256 × 256");
    harness.get_by_label("1002");
    harness.get_by_value("128 × 128*");
    click_checkbox(&mut harness, "1002");
    assert!(harness.state().session.doc.project.texture_sets[0].bake_excluded_tiles.contains(&1002));
    assert!(!harness.get_by_label("Bake Selected Textures (1)").accesskit_node().is_disabled());
    click_checkbox(&mut harness, "1001");
    assert!(harness.get_by_label("Bake Selected Textures (0)").accesskit_node().is_disabled());
    harness.state_mut().session.undo().unwrap();
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);
    click(&mut harness, "UV tiles (UDIM) (2)");
    assert!(harness.query_by_label("1001").is_none());
    assert!(harness.query_by_label("1002").is_none());
    assert_eq!(harness.state().session.doc.project, before);
}

#[test]
fn restoring_map_defaults_updates_other_sets_even_when_active_values_are_default() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.maps.ao.is_enabled = false;
    second.maps.ao.parameters.rays.rays = 16;
    second.maps.ao.parameters.ground_plane.offset = 3.0;
    let before = session.doc.project.clone();

    click(&mut harness, "Restore Defaults");

    let mut expected = before.clone();
    expected.texture_sets[1].bake.maps.ao.parameters = BakeSettings::default().maps.ao.parameters;
    let session = &mut harness.state_mut().session;
    assert_eq!(session.doc.project, expected);
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
}

#[test]
fn restoring_common_defaults_keeps_each_sets_inclusion_and_maps() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    session.doc.project.texture_sets[0].bake.common = Default::default();
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.common.resolution_height = Some(BakeResolution::R256);
    second.common.apply_diffusion = false;
    second.maps.select([BakedMap::Height]);
    second.high_poly.low_suffix = "_custom_low".into();
    let before = session.doc.project.clone();

    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_all_by_role_and_label(Role::Button, "Restore Defaults").next().unwrap().click();
    harness.run_steps(3);

    let mut expected = before.clone();
    expected.texture_sets[1].bake.common = Default::default();
    expected.texture_sets[1].bake.common.is_enabled = false;
    let session = &mut harness.state_mut().session;
    assert_eq!(session.doc.project, expected);
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
}

#[test]
fn output_axes_unlock_independently_and_relock_to_width() {
    let mut harness = harness(None);
    harness.get_by_label("Texture Sets");
    harness.get_by_label("Mesh Map Settings");
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    let width = harness.state().session.doc.project.texture_sets[0].bake.common.resolution.pixels().to_string();
    assert!(harness.get_all_by_value(&width).nth(1).unwrap().accesskit_node().is_disabled());

    click(&mut harness, "Lock Output Size");
    assert!(!harness.get_all_by_value(&width).nth(1).unwrap().accesskit_node().is_disabled());
    harness.get_all_by_value(&width).next().unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "128");
    let settings = &harness.state().session.doc.project.texture_sets[0].bake.common;
    assert_eq!(settings.resolution, BakeResolution::R128);
    assert_eq!(settings.resolution_height.map(BakeResolution::pixels), Some(width.parse().unwrap()));

    click(&mut harness, "Lock Output Size");
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.common.output_size(), (128, 128));
    assert!(harness.get_all_by_value("128").nth(1).unwrap().accesskit_node().is_disabled());
    harness.state_mut().ui.dock.focus(Panel::MeshMapSettings);
    harness.run_steps(3);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    assert!(harness.get_all_by_value("128").nth(1).unwrap().accesskit_node().is_disabled());

    harness.state_mut().ui.mode = Mode::Painting;
    harness.run_steps(3);
    assert!(!harness.state().ui.dock.is_open(Panel::CommonSettings));
    assert!(harness.state().ui.dock.is_open(Panel::Assets));
}

#[test]
fn height_controls_follow_manual_normalization_and_the_row_checkbox_enables_height() {
    let mut harness = harness(None);
    assert!(harness.query_by_label("Normalization").is_none());
    select_map(&mut harness, "Height");
    assert!(!harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled, "selecting is not enabling");
    assert!(harness.query_by_label("Scaling Divisor").is_none());
    click(&mut harness, "Normalization");
    click(&mut harness, "Manual");
    harness.get_by_label("Scaling Divisor");
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.parameters.normalization, HeightNormalization::Manual);

    click_checkbox(&mut harness, "Height");
    assert!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled);
    click(&mut harness, "Restore Defaults");
    assert!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled, "Restore Defaults keeps the map enabled");
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.parameters.normalization, HeightNormalization::RelativeToLowMesh);
    assert!(harness.query_by_label("Scaling Divisor").is_none());
    click_checkbox(&mut harness, "Height");
    assert!(!harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled);
}

#[test]
fn clicking_a_row_shows_that_maps_settings_without_changing_what_bakes() {
    let mut harness = harness(Some(BakedMap::Curvature));
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    let before = enabled_maps(&harness);

    select_map(&mut harness, "Ambient Occlusion");
    assert!(harness.state().ui.dock.is_visible(Panel::MeshMapSettings), "selecting a map fronts its settings");
    harness.get_by_label("Attenuation");
    assert!(harness.query_by_label("Method").is_none());
    select_map(&mut harness, "Opacity");
    harness.get_by_label("No Parameters");
    assert!(harness.query_by_label("Attenuation").is_none());
    assert_eq!(enabled_maps(&harness), before);

    click(&mut harness, &format!("{} Common Settings", egui_phosphor::regular::GEAR));
    assert!(harness.state().ui.dock.is_visible(Panel::CommonSettings));
    assert_eq!(enabled_maps(&harness), before);
}

#[test]
fn check_all_enables_every_map_then_disables_them_all() {
    let mut harness = harness(Some(BakedMap::Curvature));
    click_checkbox(&mut harness, "Check All");
    assert_eq!(enabled_maps(&harness), BakedMap::ALL);
    click_checkbox(&mut harness, "Check All");
    assert!(enabled_maps(&harness).is_empty());
}

#[test]
fn the_breadcrumb_switches_the_shown_map() {
    let mut harness = harness(Some(BakedMap::Curvature));
    harness.get_by_label("Sampling Radius");
    assert_eq!(harness.get_all_by_role_and_label(Role::Button, "Position").count(), 1, "only the list row before the menu opens");

    let dropdowns: Vec<_> = harness.get_all_by_label(egui_phosphor::regular::CARET_RIGHT).collect();
    let [_texture_set_dropdown, map_dropdown] = dropdowns.as_slice() else { panic!("the breadcrumb has a texture set and a map dropdown") };
    map_dropdown.click();
    harness.run_steps(3);
    assert_eq!(harness.get_all_by_role_and_label(Role::Button, "Position").count(), 2, "the map menu lists Position");

    // The menu draws its items after the list rows
    harness.get_all_by_role_and_label(Role::Button, "Position").last().unwrap().click();
    harness.run_steps(3);
    harness.get_by_label("Normalization Scale");
    assert!(harness.query_by_label("Sampling Radius").is_none());
    assert_eq!(enabled_maps(&harness), [BakedMap::Curvature]);
}

#[test]
fn curvature_controls_follow_the_method_and_notes_visibility_rules() {
    let mut harness = harness(Some(BakedMap::Curvature));
    let curvature = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.texture_sets[0].bake.maps.curvature.parameters;

    assert_eq!(curvature(&harness).method, CurvatureMethod::Mesh);
    assert!(harness.query_by_label("Details").is_none());
    harness.get_by_label("Sampling Radius");
    assert!(harness.query_by_label("Value Bounds Min").is_none());
    assert!(harness.query_by_label("Value Bounds Max").is_none());

    click(&mut harness, "Auto Tonemapping");
    harness.get_by_label("Value Bounds Min");
    harness.get_by_label("Value Bounds Max");
    assert!(!curvature(&harness).mesh.auto_tonemapping);

    click(&mut harness, "Method");
    click(&mut harness, "Generate from Normal Map");
    assert_eq!(curvature(&harness).method, CurvatureMethod::NormalMap);
    assert!(harness.query_by_label("Sampling Radius").is_none());
    harness.get_by_label("Details");

    harness.get_by_label("Seams Intensity");
    click(&mut harness, "Enable Seams");
    assert!(harness.query_by_label("Seams Intensity").is_none());
}

#[test]
fn cage_and_skew_controls_require_a_high_poly_and_follow_cage_type() {
    let mut harness = harness(None);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    assert!(harness.query_by_label("Cage Type").is_none());
    assert!(harness.query_by_label("Skew Correction").is_none());

    harness.state_mut().session.add_high_poly_bytes("High.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    harness.run_steps(3);
    harness.get_by_label("Max Frontal Distance");
    harness.get_by_label("Skew Correction");

    click(&mut harness, "Cage Type");
    click(&mut harness, "Custom File");
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.cage.kind, CageType::CustomFile);
    harness.get_by_label("Cage Scene");
    assert!(harness.query_by_label("Max Frontal Distance").is_none());
    assert!(harness.query_by_label("Skew Correction").is_none());

    for button_size in [20.0, 28.0] {
        harness.ctx.all_styles_mut(|style| style.spacing.interact_size = egui::Vec2::splat(button_size));
        harness.run_steps(3);
        for label in ["Remove", "Load Cage"] {
            assert_eq!(harness.get_by_label(label).rect().size(), egui::Vec2::splat(button_size));
        }
    }
}
