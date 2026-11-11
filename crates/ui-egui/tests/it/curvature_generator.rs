use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{CurvatureMode, EffectKind, LayerId};
use texturecraft_engine::{ViewMode, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

fn harness(render: bool) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(512, 256).unwrap();
    session.active_set = 1;
    session.camera.pitch = 0.5;
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({})).unwrap();
    let effect = execute(&mut session, "effect.add.generator", &json!({"generator": "curvature"})).unwrap()["effect"].as_u64().unwrap();
    session.view = ViewMode::Mask(LayerId(layer));
    session.update();
    let builder = Harness::builder().with_size(egui::vec2(1600.0, 1200.0));
    let builder = if render { builder.wgpu() } else { builder };
    let mut harness = builder.build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.close(Panel::Layers);
    harness.state_mut().ui.dock.close(Panel::TextureSetSettings);
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);
    (harness, LayerId(effect))
}

fn select_mode(harness: &mut Harness<'_, TextureCraftApp>, current: CurvatureMode, next: CurvatureMode) {
    harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(current.label())).unwrap().click();
    harness.run_steps(2);
    harness.get_by_label(next.label()).click();
    harness.run_steps(4);
}

#[test]
fn mode_and_detail_controls_apply_and_undo() {
    let (mut harness, effect) = harness(false);
    select_mode(&mut harness, CurvatureMode::Edges, CurvatureMode::Cavities);
    let detail = |harness: &Harness<'_, TextureCraftApp>| match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Generator(generator) => generator.detail.clone().unwrap(),
        _ => panic!("expected a curvature generator"),
    };
    assert_eq!(detail(&harness).mode, CurvatureMode::Cavities);
    let previous = detail(&harness);
    let history = harness.state().session.history_labels().len();
    harness.get_by_label("8 px").scroll_to_me();
    harness.run_steps(3);
    let label = harness.get_by_label("8 px").rect();
    let start = egui::pos2(label.left() + 20.0, label.bottom() + 10.0);
    let end = start + egui::vec2(100.0, 0.0);
    harness.drag_at(start);
    harness.run_steps(2);
    harness.hover_at(end);
    harness.run_steps(2);
    harness.drop_at(end);
    harness.run_steps(3);
    assert_ne!(detail(&harness).weights[4], previous.weights[4]);
    assert_eq!(harness.state().session.history_labels().len(), history + 1);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(detail(&harness), previous);
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn curvature_mode_snapshots() {
    let directory = std::path::PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    let (mut harness, _) = harness(true);
    eprintln!("Rendering adapter: {}", harness.state().gpu.as_ref().unwrap().adapter);
    let mut current = CurvatureMode::Edges;
    for (mode, filename) in [
        (CurvatureMode::Edges, "curvature-edges.png"),
        (CurvatureMode::Cavities, "curvature-cavities.png"),
        (CurvatureMode::Both, "curvature-both.png"),
        (CurvatureMode::Raw, "curvature-raw.png"),
    ] {
        if mode != current {
            select_mode(&mut harness, current, mode);
        }
        current = mode;
        harness.state_mut().session.update();
        harness.run_steps(12);
        harness.render().unwrap().save(directory.join(filename)).unwrap();
    }
}
