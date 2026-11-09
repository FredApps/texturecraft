use std::sync::Arc;

use serde_json::json;
use texturecraft_engine::bake::Progress;
use texturecraft_engine::core::BakedMap;
use texturecraft_engine::geom::{NO_TRI, UvGBuffer, primitives};
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut session = Session::new_project("Bake detail", primitives::udim_plane(), "builtin:udim_plane", None, 256).unwrap();
    execute(&mut session, "bake.settings", &json!({"resolution": 128, "maps": ["position"], "dilation": 0, "patch": {"common": {"apply_diffusion": false}}}))
        .unwrap();
    session
}

#[test]
fn working_tile_resolution_preserves_bake_samples_and_pixels() {
    let mut session = session();
    let original = session.bake_task(0).unwrap().run(&Progress::new()).unwrap();
    execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
    let task = session.bake_task(0).unwrap();
    let output = task.run(&Progress::new()).unwrap();
    assert_eq!((output.resolution, output.height), (256, 128));

    let gbuffer = UvGBuffer::new(&task.mesh, task.material, output.resolution, output.height);
    let mut samples = [0usize; 2];
    for (index, triangle) in gbuffer.tri.iter().enumerate() {
        if *triangle != NO_TRI {
            samples[(index % 256) / 128] += 1;
        }
    }
    assert_eq!(samples, [128 * 128; 2], "each tile receives the full Bake Output Size");
    assert_eq!(output.maps, original.maps);
}

#[test]
fn changing_working_resolution_never_resamples_stored_bakes() {
    let mut session = session();
    let output = session.bake_task(0).unwrap().run(&Progress::new()).unwrap();
    session.apply_bake(0, output).unwrap();
    let original = session.doc.baked[&0].clone();
    for resolution in [128, 512, 256] {
        execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": resolution})).unwrap();
        let current = &session.doc.baked[&0];
        assert!(Arc::ptr_eq(&current.maps[&BakedMap::Position], &original.maps[&BakedMap::Position]), "working resize must retain the bake buffer");
        assert_eq!(current, &original);
        let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
        assert_eq!(reopened.doc.baked[&0], original);
    }
    session.undo().unwrap();
    assert_eq!(session.doc.baked[&0], original);
    session.redo().unwrap();
    assert_eq!(session.doc.baked[&0], original);

    for resolution in [512, 128] {
        execute(&mut session, "textureSet.resolution", &json!({"resolution": resolution})).unwrap();
        assert!(Arc::ptr_eq(&session.doc.baked[&0].maps[&BakedMap::Position], &original.maps[&BakedMap::Position]));
    }
}

#[test]
fn rectangular_bakes_and_excluded_vertical_tiles_ignore_working_resolution() {
    let mut mesh = primitives::udim_plane();
    let second_plane = mesh.uvs.len() / 2;
    for uv in mesh.uvs.iter_mut().skip(second_plane) {
        uv.x -= 1.0;
        uv.y -= 1.0;
    }
    let mut session = Session::new_project("Vertical bake", mesh, "builtin:udim_plane", None, 256).unwrap();
    execute(
        &mut session,
        "bake.settings",
        &json!({"resolution": 64, "maps": ["position"], "dilation": 0, "patch": {"common": {"resolution_height": "R128", "apply_diffusion": false}}}),
    )
    .unwrap();
    let original = session.bake_task(0).unwrap().run(&Progress::new()).unwrap();
    assert_eq!((original.resolution, original.height), (64, 256));
    let pixels = original.maps.clone();
    session.apply_bake(0, original).unwrap();
    execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1011, "resolution": 128})).unwrap();
    execute(&mut session, "bake.tile.set", &json!({"tile": 1011, "enabled": false})).unwrap();
    let output = session.bake_task(0).unwrap().run(&Progress::new()).unwrap();
    assert_eq!(output.maps, pixels, "rebaked and excluded tiles both keep their full rectangular detail");
}

#[test]
fn imported_mesh_maps_display_and_export_the_entire_overridden_tile() {
    use texturecraft_engine::compose::{DisplayKind, tile_bytes};
    use texturecraft_engine::formats::export::{Dest, ExportOptions, ExportPreset, FileFormat, MapSpec, SourceRef};

    let mut session = session();
    execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
    let image = image::GrayImage::from_fn(128, 128, |x, y| image::Luma([(x + y) as u8]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    for tile in [1001, 1002] {
        session.import_resource_bytes(&format!("ao.{tile}.png"), bytes.clone()).unwrap();
    }
    execute(&mut session, "textureSet.meshMap.set", &json!({"map": "ambientOcclusion", "image": "ao.1001.png"})).unwrap();
    while session.update() {}

    let baked = &session.doc.baked[&0];
    let map = BakedMap::AmbientOcclusion;
    assert!((baked.scalar(map, 248, 120, (256, 128)).unwrap() - 240.0 / 255.0).abs() < 0.001);
    let composite = &session.caches[0].composite;
    let display = tile_bytes(composite, baked, DisplayKind::Orm, 5, 1);
    assert_eq!(display[((56 * 64 + 56) * 4) as usize], 240, "the viewport samples the far corner of the smaller working tile");

    let options = ExportOptions::new(ExportPreset {
        name: "Bake detail".into(),
        maps: vec![MapSpec {
            file: "ao.$udim".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::L, SourceRef::Baked { map, component: 0 })],
        }],
    });
    let files = session.export_files(&[0], &options).unwrap();
    let file = files.iter().find(|file| file.name == "ao.1002.png").unwrap();
    let exported = image::load_from_memory(&file.bytes).unwrap().to_luma8();
    assert_eq!(exported.dimensions(), (128, 128));
    assert_eq!(exported.get_pixel(120, 120)[0], 240);
    assert_eq!(exported.get_pixel(10, 10)[0], 20);

    let layer = session.doc.project.texture_sets[0].stack[0];
    execute(&mut session, "mask.add.black", &json!({"layer": layer.0})).unwrap();
    execute(&mut session, "effect.add.generator", &json!({"layer": layer.0, "generator": "ambientOcclusion"})).unwrap();
    let mask = session.mask_at(layer, &[(376, 120), (266, 10)]).unwrap();
    assert_eq!(mask, vec![1.0, 0.0], "generators sample the whole bake through the working layout");
}
