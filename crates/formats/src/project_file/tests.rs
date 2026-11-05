use std::sync::Arc;

use texturecraft_core::{BakedMap, Effect, EffectKind, Fill, Layer, LayerKind, Mask, MaskBackground, TextureSet};

use super::*;

struct Sample {
    pf: ProjectFile,
    paint: LayerId,
}

fn sample() -> Sample {
    let mut p = Project::new("Demo", "mesh.glb");
    p.texture_sets.push(TextureSet::new("A", 0, 128));
    p.texture_sets.push(TextureSet::new("B", 1, 64));
    let layer = Layer::new(p.alloc_id(), "Paint", LayerKind::Paint, &Channel::DEFAULT_SET);
    let paint = p.insert_layer(0, layer, None, false).unwrap();
    let mut masked = Layer::new(p.alloc_id(), "Masked", LayerKind::Fill(Fill::default()), &Channel::DEFAULT_SET);
    let effect = p.alloc_id();
    masked.mask = Some(Mask { effects: vec![Effect::new(effect, EffectKind::Paint)], ..Mask::new(MaskBackground::Black) });
    p.insert_layer(1, masked, None, false).unwrap();
    let mut pixels = PixelStore::default();
    let img = pixels.layer_mut(paint, Channel::BaseColor, (128, 128));
    img.set(0, 0, &[1, 2, 3, 4]);
    img.set(127, 127, &[65535, 0, 9, 65535]);
    pixels.layer_mut(paint, Channel::Height, (128, 128)).set(70, 3, &[40000, 65535]);
    pixels.mask_mut(effect, (64, 64)).set(10, 60, &[30000, 65535]);
    let ao: Vec<f32> = (0..256).map(|i| i as f32 / 255.0).collect();
    let baked = BakedMaps { resolution: 16, maps: [(BakedMap::AmbientOcclusion, Arc::new(ao)), (BakedMap::Normal, Arc::new(vec![0.5; 16 * 16 * 3]))].into() };
    let pf = ProjectFile {
        project: p,
        pixels,
        baked: [(0, baked)].into(),
        mesh: Some(("glb".into(), vec![1, 2, 3, 4, 5])),
        high_poly: vec![("b_high.obj".into(), vec![7; 9]), ("a_high.ply".into(), vec![6])],
        cage: Some(("cage.obj".into(), vec![5, 5])),
        bake_paint: [((0, texturecraft_core::BakePaintTarget::Skew), BakePaintData { width: 3, height: 2, values: vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.125] })]
            .into(),
        thumbnail: Some(vec![137, 80, 78, 71]),
        resources: [("bricks.png".to_string(), vec![9, 8, 7])].into(),
        warnings: Vec::new(),
    };
    Sample { pf, paint }
}

fn assert_same(a: &ProjectFile, b: &ProjectFile) {
    assert_eq!(a.project, b.project);
    assert_eq!(a.pixels.layers.len(), b.pixels.layers.len());
    for (k, img) in &a.pixels.layers {
        assert_eq!(Some(img), b.pixels.layers.get(k), "{k:?}");
    }
    assert_eq!(a.pixels.masks.len(), b.pixels.masks.len());
    for (k, img) in &a.pixels.masks {
        assert_eq!(Some(img), b.pixels.masks.get(k), "{k:?}");
    }
    assert_eq!(a.baked, b.baked);
    assert_eq!(a.mesh, b.mesh);
    assert_eq!((&a.high_poly, &a.cage), (&b.high_poly, &b.cage), "bake meshes keep their order");
    assert_eq!(a.bake_paint, b.bake_paint);
    assert_eq!(a.thumbnail, b.thumbnail);
    assert_eq!(a.resources, b.resources);
}

fn is_guard_error(e: &FormatError) -> bool {
    e.to_string().contains("parser failed")
}

#[test]
fn round_trip_is_identical_and_deterministic() {
    let s = sample();
    let bytes = save_project_bytes(&s.pf).unwrap();
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.warnings.is_empty());
    assert_same(&s.pf, &back);
    assert_eq!(back.pixels.layer(s.paint, Channel::BaseColor).unwrap().get(127, 127), [65535, 0, 9, 65535]);
    assert_eq!(save_project_bytes(&back).unwrap(), bytes);
    // Built-in mesh projects carry no mesh entry.
    let mut builtin = s.pf.clone();
    builtin.mesh = None;
    builtin.thumbnail = None;
    builtin.project.mesh = "builtin:sphere".into();
    assert_same(&builtin, &load_project_bytes(&save_project_bytes(&builtin).unwrap()).unwrap());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn save_and_load_through_files() {
    let s = sample();
    let dir = std::env::temp_dir().join(format!("tc-formats-project-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("demo.tcraft");
    save_project(&path, &s.pf).unwrap();
    save_project(&path, &s.pf).unwrap(); // overwrite in place
    assert!(!dir.join("demo.tcraft.tmp").exists());
    assert_same(&s.pf, &load_project(&path).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn newer_format_is_refused() {
    let mut s = sample();
    s.pf.project.format_version = FORMAT_VERSION + 1;
    let err = load_project_bytes(&save_project_bytes(&s.pf).unwrap()).unwrap_err();
    assert!(matches!(err, FormatError::NewerVersion { found, .. } if found == FORMAT_VERSION + 1), "{err}");
    assert!(err.to_string().contains("newer"));
}

#[test]
fn malformed_project_files_are_errors() {
    let bytes = save_project_bytes(&sample().pf).unwrap();
    for len in 0..bytes.len() {
        let e = load_project_bytes(&bytes[..len]).unwrap_err();
        assert!(!is_guard_error(&e), "prefix {len}: {e}");
    }
    for i in (0..bytes.len()).step_by(3) {
        let mut b = bytes.clone();
        b[i] ^= 0x5A;
        if let Err(e) = load_project_bytes(&b) {
            assert!(!is_guard_error(&e), "flip at {i}: {e}");
        }
    }
    assert!(load_project_bytes(b"PK\x03\x04 definitely not a zip").is_err());
}

/// A project zip with `project.json` from `sample()` plus raw extra entries.
fn zip_with(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let s = sample();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("project.json", SimpleFileOptions::default()).unwrap();
    zip.write_all(&serde_json::to_vec(&s.pf.project).unwrap()).unwrap();
    for (name, data) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn tile_bin(res: u32, comps: usize, tiles: &[(u32, u32)]) -> Vec<u8> {
    let mut b = header(res, res, comps, tiles.len());
    for (tx, ty) in tiles {
        b.extend(tx.to_le_bytes());
        b.extend(ty.to_le_bytes());
        b.extend(vec![7u8; TILE_AREA * comps * 2]);
    }
    b
}

#[test]
fn tiles_are_validated_against_the_texture_set() {
    let paint = sample().paint;
    let name = format!("pixels/layer_{}_baseColor.bin", paint.0);
    let ok = load_project_bytes(&zip_with(&[(&name, tile_bin(128, 4, &[(1, 1)]))])).unwrap();
    assert_eq!(ok.pixels.layer(paint, Channel::BaseColor).unwrap().get(64, 64), [0x0707; 4]);
    for (what, bin) in [
        ("tile out of range", tile_bin(128, 4, &[(2, 0)])),
        ("repeated tile", tile_bin(128, 4, &[(0, 0), (0, 0)])),
        ("wrong resolution", tile_bin(64, 4, &[(0, 0)])),
        ("wrong components", tile_bin(128, 3, &[(0, 0)])),
        ("truncated", tile_bin(128, 4, &[(0, 0)])[..100].to_vec()),
        ("trailing data", [tile_bin(128, 4, &[]), vec![1]].concat()),
    ] {
        assert!(load_project_bytes(&zip_with(&[(&name, bin)])).is_err(), "{what}");
    }
    let mut newer = tile_bin(128, 4, &[]);
    newer[4] = 9;
    assert!(matches!(load_project_bytes(&zip_with(&[(&name, newer)])), Err(FormatError::NewerVersion { .. })));
    // Pixels of a layer that is in no stack are dropped, not an error.
    let orphan = load_project_bytes(&zip_with(&[("pixels/layer_999_baseColor.bin", tile_bin(128, 4, &[(0, 0)]))])).unwrap();
    assert!(orphan.pixels.layers.is_empty());
    assert!(load_project_bytes(&zip_with(&[("pixels/layer_x_baseColor.bin", vec![])])).is_err());
    assert!(load_project_bytes(&zip_with(&[("baked/0_ambientOcclusion.bin", header(16, 16, 1, 0))])).is_err());
    assert!(load_project_bytes(&zip_with(&[("baked/0_ambientOcclusion.bin", header(100_000, 100_000, 1, 0))])).is_err());
}

#[test]
fn uv_tiled_sets_round_trip_their_atlas_pixels_and_bakes() {
    let mut s = sample();
    s.pf.project.texture_sets[0].uv_tiles = vec![1001, 1002];
    let mut pixels = PixelStore::default();
    pixels.layer_mut(s.paint, Channel::BaseColor, (256, 128)).set(200, 100, &[1, 2, 3, 4]);
    s.pf.pixels = pixels;
    let baked = BakedMaps { resolution: 32, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![0.25; 32 * 16]))].into() };
    s.pf.baked = [(0, baked)].into();
    let back = load_project_bytes(&save_project_bytes(&s.pf).unwrap()).unwrap();
    assert_same(&s.pf, &back);
    assert_eq!(back.baked[&0].height(BakedMap::AmbientOcclusion), 16);
    assert_eq!(back.pixels.layer(s.paint, Channel::BaseColor).unwrap().get(200, 100), [1, 2, 3, 4]);
    // A square image no longer fits the tiled set.
    let mut square = s.pf.clone();
    square.project.texture_sets[0].uv_tiles.clear();
    assert!(load_project_bytes(&save_project_bytes(&square).unwrap()).is_err());
}

#[test]
fn duplicate_entry_names_are_made_unique_on_save() {
    let mut pf = sample().pf;
    pf.high_poly = vec![("model.obj".into(), vec![1]), ("model.obj".into(), vec![2])];
    pf.resources = [("a b".to_string(), vec![3]), ("a_b".to_string(), vec![4])].into();
    let bytes = save_project_bytes(&pf).expect("save with duplicate entry names");
    let back = load_project_bytes(&bytes).unwrap();
    assert_eq!(back.high_poly.len(), 2);
    let mut hp: Vec<_> = back.high_poly.iter().map(|(_, b)| b.clone()).collect();
    hp.sort();
    assert_eq!(hp, vec![vec![1], vec![2]]);
    assert_eq!(back.resources.len(), 2);
    let mut res: Vec<_> = back.resources.values().cloned().collect();
    res.sort();
    assert_eq!(res, vec![vec![3], vec![4]]);
}

/// Every entry of a zip in file order: (name, method, crc, sizes, the compressed bytes as stored).
/// The zip headers around them (version fields, zip64 size placeholders) are not part of this:
/// copying an entry between zips rewrites them.
#[allow(clippy::type_complexity)]
fn payloads(zip: &[u8]) -> Vec<(String, CompressionMethod, u32, u64, u64, Vec<u8>)> {
    let mut a = ZipArchive::new(Cursor::new(zip)).unwrap();
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index_raw(i).unwrap();
            let mut raw = Vec::new();
            f.read_to_end(&mut raw).unwrap();
            (f.name().unwrap().into_owned(), f.compression(), f.crc32(), f.size(), f.compressed_size(), raw)
        })
        .collect()
}

/// A project with several baked maps and pixel layers, big enough that entries stream in chunks.
fn big_sample() -> ProjectFile {
    let mut s = sample();
    let mut x = 12345u32;
    let mut noise = |n: usize| -> Vec<f32> {
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (i as f32 * 0.001).sin() * 0.5 + 0.5 + (x >> 24) as f32 * 1e-4
            })
            .collect()
    };
    let r = 300usize;
    let maps = [BakedMap::AmbientOcclusion, BakedMap::Normal, BakedMap::Curvature, BakedMap::Position]
        .into_iter()
        .map(|m| (m, Arc::new(noise(r * r * m.components()))))
        .collect();
    s.pf.baked.insert(1, BakedMaps { resolution: r as u32, maps });
    s.pf.mesh = Some(("obj".into(), noise(5000).iter().map(|v| (v * 200.0) as u8).collect()));
    s.pf
}

fn serial_bytes(pf: &ProjectFile) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    entries::write_serial(&mut zip, entries(pf).unwrap()).unwrap();
    zip.finish().unwrap().into_inner()
}

fn parallel_bytes(pf: &ProjectFile, workers: usize, budget: u64) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    entries::parallel::write(&mut zip, entries(pf).unwrap(), workers, budget).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn parallel_save_writes_the_same_entries_as_serial() {
    let pf = big_sample();
    let serial = serial_bytes(&pf);
    let want = payloads(&serial);
    assert!(want.len() > 10);
    for (workers, budget) in [(0, 1), (2, u64::MAX), (4, u64::MAX), (8, 1), (3, 100_000)] {
        let got = parallel_bytes(&pf, workers, budget);
        assert!(payloads(&got) == want, "{workers} workers, budget {budget}");
        assert_same(&pf, &load_project_bytes(&got).unwrap());
    }
    // The public entry point (threads chosen by the machine) agrees too, and loads back.
    assert!(payloads(&save_project_bytes(&pf).unwrap()) == want);
    assert_same(&pf, &load_project_bytes(&serial).unwrap());
}

/// The files written before entries were compressed in parallel: each entry written in one go.
fn reference_one_shot(pf: &ProjectFile) -> Vec<u8> {
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(true);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("project.json", deflate).unwrap();
    zip.write_all(&serde_json::to_vec_pretty(&pf.project).unwrap()).unwrap();
    for (ts, maps) in &pf.baked {
        for (m, data) in &maps.maps {
            zip.start_file(format!("baked/{ts}_{}.bin", m.id()), deflate).unwrap();
            zip.write_all(&header(maps.resolution, maps.height(*m), m.components(), 0)).unwrap();
            let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
            zip.write_all(&bytes).unwrap();
        }
    }
    zip.finish().unwrap().into_inner()
}

/// Streaming a baked map into the compressor in chunks may place deflate block boundaries a few
/// bytes differently than one big write, so the stored bytes can differ slightly; what the entry
/// holds (name, method, CRC, size) must not, and the compressed size stays within 0.01%.
#[test]
fn baked_entries_hold_what_the_one_shot_writer_wrote() {
    let mut pf = big_sample();
    (pf.mesh, pf.high_poly, pf.cage, pf.bake_paint, pf.thumbnail, pf.resources) = (None, vec![], None, BTreeMap::new(), None, BTreeMap::new());
    pf.pixels = PixelStore::default();
    let (new, old) = (payloads(&save_project_bytes(&pf).unwrap()), payloads(&reference_one_shot(&pf)));
    assert_eq!(new.len(), old.len());
    for (n, o) in new.iter().zip(&old) {
        assert_eq!((&n.0, n.1, n.2, n.3), (&o.0, o.1, o.2, o.3));
        assert!(n.4.abs_diff(o.4) <= 64 + o.4 / 10_000, "{}: {} vs {}", n.0, n.4, o.4);
    }
}

#[test]
fn bake_model_options_survive_the_project_archive() {
    use texturecraft_core::{BackfaceMode, CageType, HeightNormalization};

    let mut project_file = sample().pf;
    let settings = &mut project_file.project.texture_sets[0].bake;
    settings.common.resolution_height = Some(texturecraft_core::BakeResolution::R256);
    settings.cage.kind = CageType::CustomFile;
    settings.maps.ao.parameters.ignore_backface = BackfaceMode::ByMeshName;
    settings.maps.ao.parameters.rays.min_distance = 0.02;
    settings.maps.ao.parameters.ground_plane.enabled = true;
    settings.maps.curvature.parameters.mesh.sampling_radius = 0.002;
    settings.maps.height.parameters.normalization = HeightNormalization::Manual;
    settings.maps.height.parameters.scaling_divisor = 4.0;
    settings.skew.edge_protection = true;
    settings.skew.edge_distance = 0.3;

    let bytes = save_project_bytes(&project_file).unwrap();
    assert_same(&project_file, &load_project_bytes(&bytes).unwrap());
}

fn edit_archive(project_file: &ProjectFile, mut edit: impl FnMut(&mut String, &mut Vec<u8>)) -> Vec<u8> {
    let bytes = save_project_bytes(project_file).unwrap();
    let mut source = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));

    for index in 0..source.len() {
        let mut entry = source.by_index(index).unwrap();
        let mut name = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        edit(&mut name, &mut data);
        output.start_file(name, SimpleFileOptions::default()).unwrap();
        output.write_all(&data).unwrap();
    }

    output.finish().unwrap().into_inner()
}

fn edit_project_json(project_file: &ProjectFile, mut edit: impl FnMut(&mut serde_json::Value)) -> Vec<u8> {
    edit_archive(project_file, |name, data| {
        if name == "project.json" {
            let mut json = serde_json::from_slice(data).unwrap();
            edit(&mut json);
            *data = serde_json::to_vec(&json).unwrap();
        }
    })
}

#[test]
fn recovery_loads_legacy_bake_settings_and_map_names() {
    let mut expected = sample().pf;
    let maps = expected.baked.get_mut(&0).unwrap();
    maps.maps.insert(BakedMap::WorldNormal, Arc::new(vec![0.5; 16 * 16 * 3]));
    maps.maps.insert(BakedMap::BentNormal, Arc::new(vec![0.5; 16 * 16 * 3]));
    expected.project.texture_sets[0].baked = maps.maps.keys().copied().collect();
    expected.project.texture_sets[1].bake.common.dilation = 12;

    let bytes = edit_archive(&expected, |name, data| {
        if name == "project.json" {
            let mut json: serde_json::Value = serde_json::from_slice(data).unwrap();
            json["texture_sets"][0]["bake"] = serde_json::json!({
                "enabled": true, "resolution": 1024, "antialiasing": 1, "dilation": 16,
                "maps": ["Normal", "WorldNormal", "Id", "AmbientOcclusion", "Curvature", "Position", "Thickness"],
                "ao": {"rays": 64, "max_distance": 0.1}, "use_low_as_high": true
            });
            json["texture_sets"][0]["baked"].as_array_mut().unwrap().extend(serde_json::json!(["ColorMap", "WorldDirection"]).as_array().unwrap().clone());
            *data = serde_json::to_vec(&json).unwrap();
        }

        *name = name
            .replace("_normal.bin", "_normal_base.bin")
            .replace("_worldNormal.bin", "_world_space_normals.bin")
            .replace("_ambientOcclusion.bin", "_ambient_occlusion.bin")
            .replace("_bentNormal.bin", "_bent_normals.bin");
    });

    let recovered = load_project_bytes(&bytes).unwrap();
    assert_eq!(recovered.warnings.len(), 1);
    assert_same(&expected, &recovered);
    let reopened = load_project_bytes(&save_project_bytes(&recovered).unwrap()).unwrap();
    assert_same(&expected, &reopened);
    assert!(reopened.warnings.is_empty());
}

#[test]
fn recovery_keeps_readable_bake_fields_and_project_content() {
    let mut expected = sample().pf;
    let settings = &mut expected.project.texture_sets[0].bake;
    settings.common.dilation = 19;
    settings.maps.height.is_enabled = true;
    settings.maps.ao.parameters.rays.rays = 96;
    settings.maps.ao.parameters.ground_plane.enabled = true;
    settings.high_poly.low_suffix = "_lo".into();
    expected.project.texture_sets[1].bake.common.dilation = 7;

    let bytes = edit_project_json(&expected, |json| {
        let bake = &mut json["texture_sets"][0]["bake"];
        bake["common"]["antialiasing"] = serde_json::json!("unknown");
        bake["common"]["resolution_height"] = serde_json::json!(false);
        bake["maps"]["normal"] = serde_json::json!("Normal");
        bake["maps"]["ao"]["parameters"]["rays"]["max_distance"] = serde_json::json!("bad");
        bake["maps"]["opacity"]["is_enabled"] = serde_json::json!(null);
        bake["cage"] = serde_json::json!([1, 2]);
    });

    assert_same(&expected, &load_project_bytes(&bytes).unwrap());
}

#[test]
fn recovery_defaults_unreadable_bake_blocks() {
    let expected = sample().pf;
    for invalid in [serde_json::Value::Null, serde_json::json!(false), serde_json::json!("old"), serde_json::json!(["Normal"])] {
        let bytes = edit_project_json(&expected, |json| json["texture_sets"][0]["bake"] = invalid.clone());
        assert_same(&expected, &load_project_bytes(&bytes).unwrap());
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_failing_entry_is_an_error_not_a_hang() {
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for fail_at in [0usize, 3, 9] {
        let list: Vec<Entry<'_>> = (0..10)
            .map(|i| Entry {
                name: format!("e{i}"),
                options: opts,
                weight: 10,
                body: Box::new(move |w| {
                    if i == fail_at {
                        return Err(std::io::Error::other("boom"));
                    }
                    if i == 5 && fail_at == 9 {
                        panic!("worker panic");
                    }
                    w.write_all(&[i as u8; 100])
                }),
            })
            .collect();
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        assert!(entries::parallel::write(&mut zip, list, 4, 25).is_err(), "fail at {fail_at}");
    }
}

#[test]
fn recovery_skips_unsupported_baked_map_entries() {
    let expected = sample().pf;
    let bytes = edit_archive(&expected, |name, _| {
        if name == "thumbnail.png" {
            *name = "baked/0_color_map.bin".into();
        }
    });
    let mut expected = expected;
    expected.thumbnail = None;
    let recovered = load_project_bytes(&bytes).unwrap();
    assert_same(&expected, &recovered);
    assert_eq!(recovered.warnings.len(), 1);
}

#[test]
fn recovery_does_not_hide_invalid_project_structure_or_newer_versions() {
    let project_file = sample().pf;
    for (field, value) in [("resolution", serde_json::json!("bad")), ("stack", serde_json::json!([999])), ("channels", serde_json::json!(null))] {
        let bytes = edit_project_json(&project_file, |json| {
            json["texture_sets"][0]["bake"] = serde_json::json!("old");
            json["texture_sets"][0][field] = value.clone();
        });
        assert!(load_project_bytes(&bytes).is_err(), "{field}");
    }

    let bytes = edit_project_json(&project_file, |json| {
        json["format_version"] = serde_json::json!(FORMAT_VERSION + 1);
        json["texture_sets"][0]["bake"] = serde_json::json!("old");
    });
    assert!(matches!(load_project_bytes(&bytes), Err(FormatError::NewerVersion { .. })));
}

#[test]
fn recovery_still_rejects_corrupt_pixels() {
    let project_file = sample().pf;
    let bytes = edit_archive(&project_file, |name, data| {
        if name == "project.json" {
            let mut json: serde_json::Value = serde_json::from_slice(data).unwrap();
            json["texture_sets"][0]["bake"]["maps"] = serde_json::json!(["Normal"]);
            *data = serde_json::to_vec(&json).unwrap();
        } else if name.starts_with("pixels/") {
            data.truncate(10);
        }
    });

    let error = load_project_bytes(&bytes).unwrap_err();
    assert!(error.to_string().contains("pixels/"), "{error}");
}

#[test]
fn recovery_does_not_accept_unrelated_json_errors() {
    let bytes = edit_archive(&sample().pf, |name, data| {
        if name == "project.json" {
            data.splice(1..1, br#""name":"duplicate", "#.iter().copied());
        }
    });

    let error = load_project_bytes(&bytes).unwrap_err();
    assert!(error.to_string().contains("duplicate field"), "{error}");
}
