//! Validation-only whole-window captures for the live bake preview.
//! Run on hosted CI with TC_UI_SNAPSHOTS set and --ignored --nocapture.

use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering, mpsc};

use egui_kittest::Harness;
use texturecraft_engine::bake::Progress;
use texturecraft_engine::core::{BakeResolution, BakedMap};
use texturecraft_engine::{PreviewShow, ViewMode};
use texturecraft_ui_egui::state::Mode;
use texturecraft_ui_egui::{BakeJob, RunningBake, TextureCraftApp};

const MAPS: [BakedMap; 3] = [BakedMap::Id, BakedMap::AmbientOcclusion, BakedMap::Height];

fn save(harness: &mut Harness<'_, TextureCraftApp>, directory: &Path, name: &str) {
    harness.run_steps(6);
    let image = harness.render().unwrap_or_else(|error| panic!("{name}: {error}"));
    let path = directory.join(format!("{name}.png"));
    image.save(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore]
fn live_bake_window_snapshots() {
    let output_dir = PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS to the capture directory"));
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for texture_set in &mut session.doc.project.texture_sets {
        texture_set.bake = Default::default();
        texture_set.bake.common.resolution = BakeResolution::R64;
        texture_set.bake.maps.ao.parameters.rays.rays = 8;
        texture_set.bake.maps.select(MAPS);
    }
    session.view = ViewMode::UvChecker;
    let before = session.doc.baked.clone();

    // Produce real CPU maps before rendering. Held channels make each captured state stable,
    // independent of worker timing, while still exercising the app's real polling/render path.
    let mut task = session.bake_task(0).unwrap();
    task.tracer = None;
    let mut finished_maps = Vec::new();
    let output = task.run_streaming(&Progress::new(), &mut |map| finished_maps.push(map)).unwrap();
    assert_eq!(finished_maps.iter().map(|map| map.map).collect::<Vec<_>>(), MAPS);
    let mut finished_maps = finished_maps.into_iter();
    let (map_sender, maps) = mpsc::channel();
    let (result_sender, receiver) = mpsc::channel();
    result_sender.send(Ok(output)).unwrap();
    let maps_done = Arc::new(AtomicUsize::new(0));
    let progress = Arc::new(Progress::new());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_eframe(|context| {
            let render_state = context.wgpu_render_state.as_ref().expect("the snapshot uses wgpu");
            eprintln!("snapshot adapter: {:?}", render_state.adapter.get_info());
            TextureCraftApp::new(context, session)
        });
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(3);
    {
        let app = harness.state_mut();
        app.start_bake(vec![0]);
        app.session.bake_preview_set_current(0);
        app.bake = Some(BakeJob {
            queue: Vec::new(),
            total: 1,
            current: Some(RunningBake {
                set_index: 0,
                progress: progress.clone(),
                receiver,
                maps,
                maps_done: maps_done.clone(),
                maps_total: MAPS.len(),
            }),
        });
    }

    map_sender.send(finished_maps.next().unwrap()).unwrap();
    maps_done.store(1, Ordering::Relaxed);
    save(&mut harness, &output_dir, "live-bake-01-follow-finished-id");
    assert_eq!(harness.state().session.view, ViewMode::MeshMap(BakedMap::Id));
    assert_eq!(harness.state().session.bake_preview.as_ref().unwrap().show, PreviewShow::Follow);
    assert!(harness.state().session.bake_preview_baked(0).unwrap().maps.contains_key(&BakedMap::Id));
    assert!(harness.state().bake.is_some());
    assert_eq!(harness.state().session.doc.baked, before);

    harness.state_mut().session.bake_preview_show(PreviewShow::Pin(BakedMap::Id)).unwrap();
    map_sender.send(finished_maps.next().unwrap()).unwrap();
    maps_done.store(2, Ordering::Relaxed);
    save(&mut harness, &output_dir, "live-bake-02-pinned-id-after-ao");
    assert_eq!(harness.state().session.view, ViewMode::MeshMap(BakedMap::Id));
    assert_eq!(harness.state().session.bake_preview.as_ref().unwrap().show, PreviewShow::Pin(BakedMap::Id));
    assert!(harness.state().session.bake_preview_baked(0).unwrap().maps.contains_key(&BakedMap::AmbientOcclusion));
    assert_eq!(harness.state().session.doc.baked, before);

    harness.state_mut().session.bake_preview_show(PreviewShow::Material).unwrap();
    map_sender.send(finished_maps.next().unwrap()).unwrap();
    maps_done.store(3, Ordering::Relaxed);
    assert!(finished_maps.next().is_none());
    save(&mut harness, &output_dir, "live-bake-03-material");
    assert_eq!(harness.state().session.view, ViewMode::Material);
    assert_eq!(harness.state().session.bake_preview.as_ref().unwrap().show, PreviewShow::Material);
    assert!(MAPS.iter().all(|map| harness.state().session.bake_preview_baked(0).unwrap().maps.contains_key(map)));
    assert!(harness.state().bake.is_some(), "the ready result must wait for the open map stream");
    assert_eq!(harness.state().session.doc.baked, before);

    harness.state_mut().cancel_bake();
    save(&mut harness, &output_dir, "live-bake-04-canceled-uv-checker");
    assert_eq!(harness.state().session.view, ViewMode::UvChecker);
    assert!(!harness.state().session.bake_preview_active());
    assert!(harness.state().bake.is_none());
    assert!(progress.cancel.load(Ordering::Relaxed));
    assert_eq!(harness.state().session.doc.baked, before);
    // Explicitly retain the stream until cancellation has discarded the uncommitted result.
    drop(map_sender);
    drop(result_sender);
}
