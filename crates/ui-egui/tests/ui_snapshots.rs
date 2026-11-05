//! Offscreen UI snapshots at 1600x960, except the taller mesh-map panel
//! Requires a GPU adapter
//!
//! Run with TC_UI_SNAPSHOTS=<dir> cargo test -p texturecraft-ui-egui --release --test ui_snapshots -- --ignored

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{BakeResolution, BakedMap, CurvatureMethod, HeightNormalization};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;
use texturecraft_ui_egui::theme::Brightness;
use texturecraft_ui_egui::titlebar::TitleBar;

fn save(h: &mut Harness<'_, TextureCraftApp>, dir: &Path, name: &str) {
    h.run_steps(6);
    let img = h.render().unwrap_or_else(|e| panic!("{name}: {e}"));
    let path = dir.join(format!("{name}.png"));
    img.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore]
fn window_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else {
        eprintln!("set TC_UI_SNAPSHOTS=<dir> to write the pictures");
        return;
    };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(1024, 256).unwrap();
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
    for (brightness, name) in [(Brightness::MediumDark, "main"), (Brightness::Dark, "brightness-dark"), (Brightness::Light, "brightness-light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, name);
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.state_mut().title_bar = TitleBar::Integrated;
    save(&mut harness, &output_dir, "titlebar-macos");
    harness.state_mut().title_bar = TitleBar::App(craft_window::Layout::default());
    save(&mut harness, &output_dir, "titlebar-app");

    // Check hover feedback and fractional scaling
    harness.hover_at(egui::pos2(1600.0 - 23.0, 14.0));
    save(&mut harness, &output_dir, "titlebar-app-close-hover");
    harness.hover_at(egui::pos2(800.0, 500.0));
    harness.set_pixels_per_point(1.5);
    save(&mut harness, &output_dir, "titlebar-app-150");
    harness.set_pixels_per_point(1.0);
    harness.state_mut().title_bar = TitleBar::System;

    for texture_set in &mut harness.state_mut().session.doc.project.texture_sets {
        texture_set.bake = Default::default();
    }
    harness.state_mut().ui.mode = Mode::Baking;
    save(&mut harness, &output_dir, "mode-bake");
    harness.state_mut().ui.dock.focus(Panel::BakeMaps);
    harness.run_steps(3);
    harness.get_by_label("Ambient Occlusion").scroll_to_me();
    save(&mut harness, &output_dir, "mode-bake-maps");
    harness.state_mut().ui.prefs.brightness = Brightness::Light;
    save(&mut harness, &output_dir, "mode-bake-maps-light");
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.get_by_label("Opacity").scroll_to_me();
    save(&mut harness, &output_dir, "mode-bake-unchecked");
    harness.state_mut().ui.dock.focus(Panel::TextureSetList);
    save(&mut harness, &output_dir, "bake-texture-sets");
    harness.state_mut().ui.dock.focus(Panel::BakeSettings);
    save(&mut harness, &output_dir, "bake-settings");
    harness.state_mut().session.add_high_poly_bytes("High Poly.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    save(&mut harness, &output_dir, "bake-settings-high-poly");
    harness.get_by_label("Cage").scroll_to_me();
    save(&mut harness, &output_dir, "bake-settings-cage-skew");
    harness.get_by_label("Edge Contrast").scroll_to_me();
    save(&mut harness, &output_dir, "bake-settings-skew");
    harness.get_by_label("Output Size").scroll_to_me();
    harness.state_mut().session.doc.project.texture_sets[0].bake.common.resolution_height = Some(BakeResolution::R1024);
    save(&mut harness, &output_dir, "bake-settings-unlocked");
    harness.set_pixels_per_point(1.5);
    save(&mut harness, &output_dir, "bake-settings-150");
    harness.set_pixels_per_point(1.0);
    {
        let settings = &mut harness.state_mut().session.doc.project.texture_sets[0].bake;
        settings.maps.select([BakedMap::Curvature, BakedMap::Height]);
        settings.maps.curvature.parameters.method = CurvatureMethod::Mesh;
        settings.maps.height.parameters.normalization = HeightNormalization::Manual;
        settings.maps.height.parameters.scaling_divisor = 100.0;
    }
    harness.state_mut().ui.dock.focus(Panel::BakeMaps);
    harness.run_steps(3);
    harness.get_by_label("Curvature").scroll_to_me();
    save(&mut harness, &output_dir, "bake-curvature-mesh");
    harness.get_by_label("Scaling Divisor").scroll_to_me();
    save(&mut harness, &output_dir, "bake-height-manual");
    harness.state_mut().session.bake_meshes.high.clear();
    harness.state_mut().ui.mode = Mode::Painting;
    save(&mut harness, &output_dir, "painting-restored");

    harness.state_mut().ui.dock.focus(Panel::Display);
    save(&mut harness, &output_dir, "panel-display");
    harness.state_mut().ui.dock.close(Panel::Display);
    harness.state_mut().ui.windows.export = true;
    save(&mut harness, &output_dir, "window-export");
    harness.state_mut().ui.windows.export = false;
    harness.state_mut().ui.windows.settings = true;
    save(&mut harness, &output_dir, "window-settings");
    harness.state_mut().ui.windows.settings = false;

    // A taller window exposes every mesh-map slot
    harness.state_mut().ui.dock.focus(Panel::TextureSetSettings);
    harness.set_size(egui::vec2(1600.0, 1800.0));
    save(&mut harness, &output_dir, "panel-texture-set-settings");
    harness.set_size(egui::vec2(1600.0, 960.0));

    // Show the mask histogram in Properties
    let session = &mut harness.state_mut().session;
    for (id, params) in [
        ("layer.add.fill", serde_json::json!({"color": [0.8, 0.3, 0.1]})),
        ("mask.add.black", serde_json::Value::Null),
        ("effect.add.fill", serde_json::json!({"target": "mask", "procedural": "clouds"})),
        ("effect.add.levels", serde_json::json!({"target": "mask", "inLow": 0.3, "inHigh": 0.8})),
    ] {
        texturecraft_engine::execute(session, id, &params).unwrap();
    }
    save(&mut harness, &output_dir, "properties-levels");

    // Show the second path selected in Properties
    let session = &mut harness.state_mut().session;
    for (id, params) in [
        ("layer.add.paint", serde_json::Value::Null),
        ("tool.path", serde_json::json!({"uvPoints": [[0.2, 0.5], [0.5, 0.4], [0.8, 0.5]], "commit": true})),
        ("tool.path", serde_json::json!({"uvPoints": [[0.3, 0.7], [0.7, 0.7], [0.7, 0.9]], "closed": true, "fill": true, "commit": true})),
    ] {
        texturecraft_engine::execute(session, id, &params).unwrap();
    }

    let effect = match session.target {
        texturecraft_engine::StackTarget::Effect(e) => e.0,
        _ => 0,
    };
    texturecraft_engine::execute(session, "path.edit", &serde_json::json!({"effect": effect, "index": 1})).unwrap();
    save(&mut harness, &output_dir, "properties-paths");

    // Show the brush cursor and readout during Ctrl+right drag
    let cursor_position = harness.state().viewport_rect.center();
    let (modifiers, button) = (egui::Modifiers::COMMAND, egui::PointerButton::Secondary);
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.event(egui::Event::PointerButton { pos: cursor_position, button, pressed: true, modifiers });
    for step in 1..=4 {
        harness.hover_at(cursor_position + egui::vec2(15.0 * step as f32, -8.0 * step as f32));
        harness.run_steps(1);
    }
    save(&mut harness, &output_dir, "brush-ctrl-drag");
    harness.event(egui::Event::PointerButton { pos: cursor_position, button, pressed: false, modifiers });
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
}
