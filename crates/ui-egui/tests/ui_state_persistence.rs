//! App save/restart coverage, including offscreen rendering when explicitly requested

use eframe::App as _;
use egui_kittest::Harness;
use texturecraft_ui_egui::{TextureCraftApp, docking::Panel, state::Mode};

#[path = "../src/ui_state_store/memory_storage.rs"]
mod memory_storage;
use memory_storage::MemoryStorage;

fn app(storage: &MemoryStorage, render: bool) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    // Existing viewport controls lack accessible names; these tests check persistence and rendering
    let builder = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_accessibility_check(false);
    let builder = if render { builder.wgpu() } else { builder };
    builder.build_eframe(|context| {
        let mut creation = eframe::CreationContext::_new_kittest(context.egui_ctx.clone());
        creation.storage = Some(storage);
        creation.wgpu_render_state = context.wgpu_render_state.clone();
        TextureCraftApp::new(&creation, session)
    })
}

fn customize(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.state_mut().ui.dock.close(Panel::Assets);
    harness.state_mut().ui.split_ratio = 0.35;
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(3);
    harness.state_mut().ui.dock.close(Panel::MeshMapBakers);
    harness.state_mut().ui.mode = Mode::Painting;
    harness.run_steps(3);
}

#[test]
fn app_save_restores_both_customized_modes() {
    let mut storage = MemoryStorage::default();
    let mut original = app(&storage, false);
    customize(&mut original);
    original.state_mut().save(&mut storage);
    let mut restarted = app(&storage, false);
    restarted.run_steps(3);
    assert_eq!(restarted.state().ui.dock, original.state().ui.dock);
    assert_eq!(restarted.state().ui.split_ratio, 0.35);

    for harness in [&mut original, &mut restarted] {
        harness.state_mut().ui.mode = Mode::Baking;
        harness.run_steps(3);
    }
    assert_eq!(restarted.state().ui.dock, original.state().ui.dock);
    assert!(!restarted.state().ui.dock.is_open(Panel::MeshMapBakers));
}

#[test]
fn failed_backups_show_one_persistent_warning_and_keep_the_original() {
    let mut storage = MemoryStorage::default();
    storage.values.insert("ui.json".into(), "not json".into());
    for index in 1..=10 {
        let key = if index == 1 { "ui.unreadable.json".into() } else { format!("ui.unreadable-{index}.json") };
        storage.values.insert(key, "previous backup".into());
    }
    let mut harness = app(&storage, false);
    harness.state_mut().save(&mut storage);
    harness.state_mut().save(&mut storage);
    assert_eq!(storage.values["ui.json"], "not json");
    let warnings = &harness.state().toasts;
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].error && warnings[0].until.is_infinite());
    assert!(warnings[0].text.contains("Saving is disabled"));
    assert!(harness.state().session.log.iter().any(|line| line == &warnings[0].text));
}

#[test]
#[ignore = "requires an offscreen GPU and TC_UI_SNAPSHOTS"]
fn customized_layout_restart_snapshots() {
    let output = std::path::PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&output).unwrap();
    let file = output.join("restart-ui.json");
    let _ = std::fs::remove_file(&file);
    texturecraft_ui_egui::ui_state_store::FILE.set(file).unwrap();
    let mut storage = MemoryStorage::default();
    let mut original = app(&storage, true);
    eprintln!("Rendering adapter: {}", original.state().gpu.as_ref().unwrap().adapter);
    customize(&mut original);
    original.state_mut().save(&mut storage);
    let mut restarted = app(&storage, true);
    eprintln!("Restart rendering adapter: {}", restarted.state().gpu.as_ref().unwrap().adapter);

    for (mode, name) in [(Mode::Painting, "painting"), (Mode::Baking, "baking")] {
        for (harness, suffix) in [(&mut original, "before"), (&mut restarted, "after")] {
            harness.state_mut().ui.mode = mode;
            harness.run_steps(8);
            harness.render().unwrap().save(output.join(format!("restart-{name}-{suffix}.png"))).unwrap();
        }
        assert_eq!(restarted.state().ui.dock, original.state().ui.dock);
        assert_eq!(restarted.state().ui.split_ratio, original.state().ui.split_ratio);
    }

    let file = texturecraft_ui_egui::ui_state_store::FILE.get().unwrap();
    std::fs::remove_file(file).unwrap();
    std::fs::create_dir(file).unwrap();
    let mut unreadable = app(&storage, true);
    assert!(unreadable.state().toasts.iter().any(|toast| toast.text.contains("Saving is disabled")));
    unreadable.run_steps(8);
    unreadable.render().unwrap().save(output.join("settings-read-warning.png")).unwrap();
    std::fs::remove_dir(file).unwrap();
}
