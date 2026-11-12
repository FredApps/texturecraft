use serde_json::{Value, json};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, command_specs, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

#[test]
fn scripted_stroke_paints_and_undo_restores() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 20, "hardness": 90})).unwrap();
    let r = execute(&mut s, "tool.stroke", &json!({"points": [[180, 200], [220, 200]]})).unwrap();
    assert!(r["stamps"].as_u64().unwrap() > 1);
    // The texel under the screen center is now red.
    let hit = s.pick(texturecraft_engine::geom::glam::Vec2::new(200.0, 200.0), s.viewport).unwrap();
    let res = 256.0;
    let (x, y) = ((hit.uv.x * res) as u32, (hit.uv.y * res) as u32);
    let v = s.caches[0].composite.value(Channel::BaseColor, x, y);
    assert!(v[0] > 0.9 && v[1] < 0.1, "{v:?}");
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    s.update();
    let v = s.caches[0].composite.value(Channel::BaseColor, x, y);
    assert!((v[0] - 0.5).abs() < 0.01, "{v:?}");
    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    s.update();
    assert!(s.caches[0].composite.value(Channel::BaseColor, x, y)[0] > 0.9);
}

#[test]
fn layers_masks_materials_and_bake() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": "#336699", "roughness": 0.2})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.generator", &json!({"generator": "position", "level": 0.5})).unwrap();
    execute(&mut s, "bake.run", &json!({"resolution": 128, "rays": 8})).unwrap();
    execute(&mut s, "material.apply", &json!({"material": "steel_painted_worn"})).unwrap();
    execute(&mut s, "layer.group", &Value::Null).unwrap();
    execute(&mut s, "layer.duplicate", &Value::Null).unwrap();
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    assert!(inspect["layers"].as_array().unwrap().len() >= 3, "{inspect}");
    s.doc.project.validate().unwrap();
    s.update();
    for _ in 0..7 {
        execute(&mut s, "edit.undo", &Value::Null).unwrap();
    }
    assert_eq!(s.doc.project.walk(0).len(), 1, "back to the initial paint layer");
}

#[test]
fn generator_offset_is_settable_and_clamped() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    let r = execute(&mut s, "effect.add.generator", &json!({"generator": "position", "offset": 0.25})).unwrap();
    let eff = r["effect"].as_u64().unwrap();
    let offset = |s: &Session| {
        let layer = s.doc.project.layers.values().find(|l| l.mask.is_some()).unwrap();
        layer
            .mask
            .as_ref()
            .unwrap()
            .effects
            .iter()
            .find_map(|e| match &e.kind {
                texturecraft_engine::core::EffectKind::Generator(g) => Some(g.offset),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(offset(&s), 0.25);
    execute(&mut s, "effect.set", &json!({"effect": eff, "offset": 5.0})).unwrap();
    assert_eq!(offset(&s), 1.0, "clamped to 1");
}

fn generator_offset_of(s: &Session) -> f32 {
    let layer = s.doc.project.layers.values().find(|l| l.mask.is_some()).unwrap();
    layer
        .mask
        .as_ref()
        .unwrap()
        .effects
        .iter()
        .find_map(|e| match &e.kind {
            texturecraft_engine::core::EffectKind::Generator(g) => Some(g.offset),
            _ => None,
        })
        .unwrap()
}

#[test]
fn generator_offset_survives_save_and_reopen_and_undo_restores_it() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    let r = execute(&mut s, "effect.add.generator", &json!({"generator": "position", "offset": 0.25})).unwrap();
    let eff = r["effect"].as_u64().unwrap();
    execute(&mut s, "effect.set", &json!({"effect": eff, "offset": -0.6})).unwrap();
    assert_eq!(generator_offset_of(&s), -0.6);

    let path = std::env::temp_dir().join(format!("tc-generator-offset-{}.tcraft", std::process::id()));
    let path = path.to_str().unwrap();
    execute(&mut s, "file.saveCopy", &json!({"path": path})).unwrap();
    let reopened = texturecraft_engine::file_cmds::open_path(path);
    let _ = std::fs::remove_file(path);
    assert_eq!(generator_offset_of(&reopened.unwrap()), -0.6, "the offset is saved and read back");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(generator_offset_of(&s), 0.25, "undo restores the previous offset");
}

#[test]
fn generator_offset_works_alongside_curvature_mode_and_sizes() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    let r = execute(&mut s, "effect.add.generator", &json!({"generator": "curvature", "mode": "cavities", "offset": 0.5})).unwrap();
    let eff = r["effect"].as_u64().unwrap();
    assert_eq!(generator_offset_of(&s), 0.5);

    execute(&mut s, "effect.set", &json!({"effect": eff, "offset": -0.5, "sizes": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5]})).unwrap();
    assert_eq!(generator_offset_of(&s), -0.5);
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""offset":-0.5"#) && inspect.contains(r#""mode":"Cavities""#), "{inspect}");
    assert!(inspect.contains(r#""sizes":[0.0,0.0,0.0,1.0,0.0,0.0,0.5]"#), "{inspect}");
}

#[test]
fn layer_rename_and_undo() {
    let mut s = session();
    let layer_id = s.selected_layer().unwrap();
    let orig_name = s.doc.project.layer(layer_id).unwrap().name.clone();
    execute(&mut s, "layer.rename", &json!({"name": "Custom Paint"})).unwrap();
    assert_eq!(s.doc.project.layer(layer_id).unwrap().name, "Custom Paint");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layer(layer_id).unwrap().name, orig_name);

    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layer(layer_id).unwrap().name, "Custom Paint");
}

#[test]
fn undo_and_redo_jump_several_steps() {
    let mut s = session();
    let layer = s.selected_layer().unwrap();
    let name = |s: &Session| s.doc.project.layer(layer).unwrap().name.clone();
    let first = name(&s);
    for n in ["A", "B", "C"] {
        execute(&mut s, "layer.rename", &json!({"name": n})).unwrap();
    }
    let base = s.history_labels().len() - 3;
    // Two back to "A": the undone steps wait to be redone, next first.
    assert_eq!(execute(&mut s, "edit.undo", &json!({"steps": 2})).unwrap(), json!({"undone": "Rename layer", "steps": 2}));
    assert_eq!(name(&s), "A");
    assert_eq!(s.redo_labels().len(), 2);
    assert_eq!(execute(&mut s, "edit.history", &Value::Null).unwrap()["redo"].as_array().unwrap().len(), 2);
    // Forward past the end stops at "C"; back past the start stops at the first state.
    assert_eq!(execute(&mut s, "edit.redo", &json!({"steps": 99})).unwrap()["steps"], 2);
    assert_eq!(name(&s), "C");
    let r = execute(&mut s, "edit.undo", &json!({"steps": 1_000_000})).unwrap();
    assert_eq!(r["steps"].as_u64().unwrap() as usize, base + 3);
    assert_eq!(name(&s), first);
    assert!(execute(&mut s, "edit.undo", &json!({"steps": 2})).is_err(), "nothing left to undo");
}

#[test]
fn polygon_fill_and_color_picker() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"channel": "roughness", "value": 0.9})).unwrap();
    execute(&mut s, "tool.polygonFill", &Value::Null).unwrap();
    execute(&mut s, "tool.fillMode", &json!({"mode": "mesh"})).unwrap();
    execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).unwrap();
    execute(&mut s, "tool.colorPicker", &Value::Null).unwrap();
    let r = execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).unwrap();
    assert!((r["roughness"][0].as_f64().unwrap() - 0.9).abs() < 0.01, "{r}");
}

#[test]
fn polygon_fill_boxes_survive_hostile_params() {
    let hostile = [
        json!({"x0": 1e30, "y0": -1e30, "x1": f64::MAX, "y1": 0, "viewport": [1e9, 1e9]}),
        json!({"x0": 0, "y0": 0, "x1": 0, "y1": 0, "viewport": [16, 16]}),
        json!({"uv0": [1e30, -1e30], "uv1": [0, 0]}),
        json!({"uv0": [0.5], "uv1": "x", "uv": [f64::MIN, 2]}),
        json!({"hitTest": "nope", "visibleOnly": "yes", "mode": 7}),
    ];
    for visible_only in [true, false] {
        for hit_test in ["intersection", "containedCenter"] {
            let mut session = session();
            execute(&mut session, "tool.polygonFill", &Value::Null).unwrap();
            execute(&mut session, "tool.fillSelection", &json!({"hitTest": hit_test, "visibleOnly": visible_only})).unwrap();
            for params in &hostile {
                let _ = execute(&mut session, "tool.click", params);
                let _ = execute(&mut session, "tool.fillSelection", params);
                session.update();
            }
        }
    }
}

/// A small project still gets the production bake defaults; only panic_hunt overrides them.
#[test]
fn new_project_keeps_full_resolution_bake_defaults() {
    let s = session();
    assert!(!s.doc.project.texture_sets.is_empty());
    for texture_set in &s.doc.project.texture_sets {
        assert_eq!(texture_set.resolution, 256);
        assert_eq!(texture_set.bake, texturecraft_engine::core::BakeSettings::default());
        assert_eq!(texture_set.bake.common.output_size(), (2048, 2048));
        assert_eq!(texture_set.bake.maps.ao.parameters.rays.rays, 64);
    }
}

/// Every command must return Ok or Err — never panic — for hostile params.
#[test]
fn panic_hunt() {
    let hostile = [
        Value::Null,
        json!({}),
        json!({"layer": u64::MAX, "ts": 99, "channel": "nope", "mode": "???", "opacity": f64::MAX}),
        json!({"layer": 1, "index": -5, "delta": i64::MIN, "above": "x", "points": [[f64::MAX, -1e300]], "size": -1e30}),
        json!({"resolution": 0, "maps": ["x"], "rays": u64::MAX, "antialiasing": 3, "color": [1], "value": "nan"}),
        json!({"effect": 0, "x": 1e30, "y": -1e30, "viewport": [0, 0], "mesh": "../../etc", "material": ""}),
    ];
    for spec in command_specs() {
        if spec.id == "project.sample" {
            continue; // slow (bakes); covered elsewhere
        }
        for p in &hostile {
            let mut s = session();
            // Match the 256px fixture: otherwise empty/irrelevant bake.run params perform
            // repeated 2048px default bakes. Keep every input, ray count and fresh session.
            for texture_set in &mut s.doc.project.texture_sets {
                texture_set.bake.common.resolution = texturecraft_engine::core::BakeResolution::R256;
            }
            let _ = execute(&mut s, spec.id, p);
            s.update();
        }
    }
}

#[test]
fn copy_paste_flatten_and_resize_keep_the_look() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.9, 0.1, 0.1], "roughness": 0.3})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"procedural": "checker", "scale": 4})).unwrap();
    s.update();
    let before: Vec<[f32; 3]> = (0..256).step_by(9).map(|x| s.caches[0].composite.value(Channel::BaseColor, x, 100)).collect();
    // Flatten keeps the composite identical (within 8-bit-ish tolerance) and makes a paint layer.
    let id = s.selected_layer().unwrap();
    execute(&mut s, "layer.flatten", &json!({"layer": id.0})).unwrap();
    s.update();
    let after: Vec<[f32; 3]> = (0..256).step_by(9).map(|x| s.caches[0].composite.value(Channel::BaseColor, x, 100)).collect();
    for (a, b) in before.iter().zip(&after) {
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 2e-3, "{a:?} {b:?}");
        }
    }
    assert!(matches!(s.doc.project.layer(id).unwrap().kind, texturecraft_engine::core::LayerKind::Paint));
    // Copy into the other texture set (different resolution).
    execute(&mut s, "textureSet.resolution", &json!({"ts": 1, "resolution": 128})).unwrap();
    execute(&mut s, "layer.copy", &Value::Null).unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    let r = execute(&mut s, "layer.paste", &Value::Null).unwrap();
    let pasted = texturecraft_engine::core::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.doc.pixels.layer(pasted, Channel::BaseColor).unwrap().width(), 128);
    // Resizing resamples instead of clearing.
    execute(&mut s, "textureSet.resolution", &json!({"ts": 0, "resolution": 512})).unwrap();
    assert_eq!(s.doc.pixels.layer(id, Channel::BaseColor).unwrap().width(), 512);
    s.update();
    let big = s.caches[0].composite.value(Channel::BaseColor, 2 * 9 * 5, 200);
    let small = before[5];
    assert!((big[0] - small[0]).abs() < 0.2, "{big:?} {small:?}");
    execute(&mut s, "layer.instantiateAll", &json!({"layer": id.0})).unwrap();
    s.doc.project.validate().unwrap();
}

#[test]
fn image_resources_fill_layers_and_round_trip() {
    let mut s = session();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_fn(8, 8, |_, _| image::Rgba([255, 0, 0, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("C:/x/red bricks.png", png).unwrap();
    assert_eq!(name, "red_bricks.png");
    let r = execute(&mut s, "layer.add.image", &json!({"name": name})).unwrap();
    assert!(r["layer"].as_u64().is_some());
    s.update();
    let v = s.caches[0].composite.value(Channel::BaseColor, 128, 128);
    assert!(v[0] > 0.95 && v[1] < 0.05, "{v:?}");
    let bytes = s.to_project_bytes().unwrap();
    let back = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(back.doc.resources.len(), 1);
    assert_eq!(back.images.len(), 1);
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.images.is_empty(), "undoing the import forgets the decoded image");
    assert!(s.import_resource_bytes("bad.png", vec![1, 2, 3]).is_err());
}

#[test]
fn brush_presets_apply_save_and_load() {
    let mut s = session();
    let list = execute(&mut s, "brush.preset.list", &Value::Null).unwrap();
    assert!(list.as_array().unwrap().len() >= 12);
    execute(&mut s, "tool.brush", &json!({"size": 30})).unwrap();
    let b = execute(&mut s, "brush.preset.apply", &json!({"preset": "Scratches"})).unwrap();
    assert_eq!((b["alpha"]["library"].as_str(), b["followPath"].as_bool()), (Some("scratches"), Some(true)));
    assert!((b["size"].as_f64().unwrap() - 30.0).abs() < 1e-3, "presets keep the size");
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[160, 200], [240, 200]]})).unwrap()["stamps"].as_u64().unwrap() > 0);
    // Saved and loaded back onto another brush, the stroke returns and the size stays.
    let path = std::env::temp_dir().join(format!("tc-brush-{}.tcbrush", std::process::id()));
    let path = path.to_str().unwrap();
    execute(&mut s, "brush.preset.save", &json!({"path": path})).unwrap();
    execute(&mut s, "brush.preset.apply", &json!({"preset": "basic_soft"})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 10})).unwrap();
    let b = execute(&mut s, "brush.preset.load", &json!({"path": path})).unwrap();
    assert_eq!(b["alpha"]["library"], "scratches");
    assert!((b["size"].as_f64().unwrap() - 10.0).abs() < 1e-3);
    std::fs::write(path, b"{}").unwrap();
    assert!(execute(&mut s, "brush.preset.load", &json!({"path": path})).is_err());
    let _ = std::fs::remove_file(path);
    for bad in [json!({}), json!({"preset": "nope"})] {
        assert!(execute(&mut s, "brush.preset.apply", &bad).is_err());
    }
}

#[test]
fn text_becomes_a_brush_alpha_and_a_decal() {
    let mut s = session();
    let fonts = execute(&mut s, "text.fonts", &Value::Null).unwrap();
    assert_eq!(fonts["builtIn"][0], "Ubuntu Light");
    let r = execute(&mut s, "resource.text", &json!({"text": "TEXT\nCRAFT", "font": "ubuntu light", "size": 64})).unwrap();
    let name = r["name"].as_str().unwrap().to_string();
    assert_eq!(name, "Text_TEXT.png");
    assert!(r["width"].as_u64().unwrap() > r["height"].as_u64().unwrap() / 2, "two short lines");
    // White with the glyphs in alpha.
    let img = s.images[&name].clone();
    assert!(img.rgba.iter().any(|t| t[3] > 0.99) && img.rgba.iter().any(|t| t[3] == 0.0) && img.rgba.iter().all(|t| t[0] == 1.0));
    // As the brush alpha it paints (keeping its proportions), and it places as a decal.
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 40, "alpha": {"image": name}})).unwrap();
    assert!(execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).is_ok());
    assert!(execute(&mut s, "layer.add.decal", &json!({"image": name})).unwrap()["layer"].as_u64().is_some());
    for bad in [json!({}), json!({"text": "  "}), json!({"text": "x", "font": "No Such Font"})] {
        assert!(execute(&mut s, "resource.text", &bad).is_err(), "{bad}");
    }
}

#[test]
fn projection_tool_paints_the_stencil_colors() {
    let mut s = session();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_fn(16, 16, |x, _| if x < 8 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 255, 0, 255]) });
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("split.png", png).unwrap();
    execute(&mut s, "tool.projection", &Value::Null).unwrap();
    // Without a stencil the tool explains what's missing.
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[150, 200]]})).is_err());
    execute(&mut s, "tool.stencil", &json!({"image": name, "scale": 1.0, "tile": true})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 6, "hardness": 90})).unwrap();
    for x in [150.0f32, 250.0] {
        execute(&mut s, "tool.stroke", &json!({"points": [[x, 200.0]]})).unwrap();
    }
    let vp = s.viewport;
    let at = |s: &Session, x: f32| {
        let hit = s.pick(texturecraft_engine::geom::glam::Vec2::new(x, 200.0), vp).unwrap();
        s.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32)
    };
    let left = at(&s, 150.0);
    let right = at(&s, 250.0);
    assert!(left[0] > 0.8 && left[1] < 0.2, "left {left:?}");
    assert!(right[1] > 0.8 && right[0] < 0.2, "right {right:?}");
}

#[test]
fn smart_materials_save_and_load_with_pixels() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    let group = texturecraft_engine::materials::apply(&mut s, "steel_painted_worn").unwrap();
    let before = s.doc.project.walk(0).len();
    let bytes = s.smart_material_bytes(group, "My Steel").unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    let id = s.add_smart_material_bytes(&bytes).unwrap();
    assert_eq!(s.doc.project.layer(id).unwrap().name, "My Steel");
    let walked = s.doc.project.walk(1).len();
    assert_eq!(walked, 1 + (before - 1), "the folder and its children arrive");
    assert!(s.add_smart_material_bytes(b"junk").is_err());
    s.doc.project.validate().unwrap();
}

/// Strokes drawn frame by frame (paint, erase, a mask) commit their staged tiles, which then
/// settle at low priority to exactly what a full recomposite gives; undo restores.
#[test]
fn committed_strokes_settle_to_the_full_composite() {
    use texturecraft_engine::geom::glam::Vec2;
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 2048).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.5, 0.3], "roughness": 0.7})).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 25, "hardness": 70, "opacity": 0.8})).unwrap();
    s.update();
    let full = |s: &mut Session| {
        s.caches[0].invalidate_all();
        s.update();
        s.caches[0].composite.clone()
    };
    let original = full(&mut s);
    for step in ["paint", "eraser", "mask"] {
        match step {
            "eraser" => drop(execute(&mut s, "tool.select", &json!({"tool": "eraser"})).unwrap()),
            "mask" => {
                execute(&mut s, "tool.select", &json!({"tool": "paint"})).unwrap();
                execute(&mut s, "mask.add.white", &Value::Null).unwrap();
                s.target = texturecraft_engine::StackTarget::Mask;
                s.tool.mask_value = 0.2;
            }
            _ => {}
        }
        s.update();
        s.begin_stroke(Vec2::new(120.0, 200.0), s.viewport, 1.0).unwrap();
        for i in 1..30 {
            s.continue_stroke(Vec2::new(120.0 + i as f32 * 6.0, 200.0 + (i as f32 * 0.4).sin() * 40.0), 1.0);
            s.update();
        }
        s.end_stroke().unwrap();
        // Nothing was left dirty: the committed tiles only settle, a slice per budgeted update.
        assert!(s.caches[0].dirty.is_empty(), "{step}");
        let settling = s.caches[0].settle.len();
        assert!(settling > 32, "{step}: {settling} tiles settle");
        let t = std::cell::Cell::new(0.0);
        let clock = || {
            t.set(t.get() + 2.0);
            t.get()
        };
        s.update_for(30.0, clock);
        let left = s.pending_tiles();
        assert!(left > 0 && left < settling, "{step}: {left} of {settling} left after one slice");
        s.update();
        let settled = s.caches[0].composite.clone();
        assert_eq!(settled, full(&mut s), "{step}: settled tiles match a full recomposite");
    }
    // Mask stroke, white mask, eraser and paint strokes.
    for _ in 0..4 {
        execute(&mut s, "edit.undo", &Value::Null).unwrap();
    }
    assert_eq!(full(&mut s), original);
}

/// A resize prepared from a snapshot and applied later matches the command, keeps edits made
/// while it ran, and undoes in one step.
#[test]
fn background_resizes_match_the_command_and_keep_later_edits() {
    use texturecraft_engine::bake::Progress;
    let stroke = |s: &mut Session, y: u32| {
        execute(s, "tool.stroke", &json!({"points": [[150, y], [250, y]]})).unwrap();
    };
    let (mut a, mut b) = (session(), session());
    for s in [&mut a, &mut b] {
        execute(s, "tool.brush", &json!({"size": 10})).unwrap();
        stroke(s, 200);
    }
    execute(&mut a, "textureSet.resolution", &json!({"resolution": 512})).unwrap();
    let task = b.resize_task(0, 512).unwrap();
    let progress = Progress::new();
    let out = task.run(&progress).unwrap();
    assert_eq!(progress.fraction(), 1.0);
    assert_eq!(out.target(), (0, 512));
    b.apply_resize(out).unwrap();
    assert_eq!(a.doc.pixels.layers, b.doc.pixels.layers);
    a.update();
    b.update();
    assert_eq!(a.caches[0].composite, b.caches[0].composite);
    assert_eq!(b.caches[0].size, (512, 512));
    // A stroke painted while the resize ran is kept (the resize runs again on apply).
    let out = b.resize_task(0, 1024).unwrap().run(&Progress::new()).unwrap();
    stroke(&mut b, 260);
    let painted = |s: &Session| s.doc.pixels.layers.values().map(|i| i.allocated_count()).sum::<usize>();
    let before = painted(&b);
    b.apply_resize(out).unwrap();
    assert_eq!(b.doc.project.texture_sets[0].resolution, 1024);
    stroke(&mut a, 260);
    execute(&mut a, "textureSet.resolution", &json!({"resolution": 1024})).unwrap();
    assert_eq!(a.doc.pixels.layers, b.doc.pixels.layers);
    assert!(painted(&b) > before);
    assert_eq!(b.undo_label(), Some("Texture set size"));
    execute(&mut b, "edit.undo", &Value::Null).unwrap();
    assert_eq!((b.doc.project.texture_sets[0].resolution, painted(&b)), (512, before));
    b.update();
    assert_eq!(b.caches[0].size, (512, 512));
}

#[test]
fn curvature_generators_take_a_mode_and_detail_sizes() {
    let mut state = session();
    execute(&mut state, "layer.add.fill", &json!({})).unwrap();
    execute(&mut state, "mask.add.black", &Value::Null).unwrap();
    let added = execute(&mut state, "effect.add.generator", &json!({"generator": "curvature", "mode": "cavities"})).unwrap();
    let effect = added["effect"].clone();

    execute(&mut state, "effect.set", &json!({"effect": effect, "sizes": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5]})).unwrap();
    let inspect = execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""mode":"Cavities""#) && inspect.contains(r#""sizes":[0.0,0.0,0.0,1.0,0.0,0.0,0.5]"#), "{inspect}");

    let too_few = execute(&mut state, "effect.set", &json!({"effect": effect, "sizes": [1.0, 0.5]}));
    assert!(too_few.is_err(), "sizes takes one weight per detail size");
    let not_numbers = execute(&mut state, "effect.set", &json!({"effect": effect, "sizes": [1.0, "a", 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]}));
    assert!(not_numbers.is_err(), "a size that isn't a number is rejected, not skipped");
    let unknown = execute(&mut state, "effect.set", &json!({"effect": effect, "mode": "ridges"}));
    assert!(unknown.is_err(), "unknown modes are rejected");
    let wrong_type = execute(&mut state, "effect.set", &json!({"effect": effect, "mode": 1, "level": 0.1}));
    assert!(wrong_type.is_err(), "a mode must be a string and an invalid edit must be rejected");
    let dirt = execute(&mut state, "effect.add.generator", &json!({"generator": "dirt", "mode": "edges"}));
    assert!(dirt.is_err(), "only curvature generators have a mode");
    assert_eq!(execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string(), inspect, "failed commands leave the project unchanged");
}

#[test]
fn unparseable_color_param_returns_error() {
    let mut s = session();
    let err = execute(&mut s, "layer.add.fill", &json!({"color": "#fff"})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.add.fill`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");

    let err = execute(&mut s, "layer.fill.set", &json!({"channel": "baseColor", "color": "#123"})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.fill.set`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");

    let err = execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.material`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");

    let err = execute(&mut s, "mask.add.colorSelection", &json!({"color": "#xyz"})).unwrap_err();
    assert_eq!(err.to_string(), "`mask.add.colorSelection`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");

    let err = execute(&mut s, "display.set", &json!({"scatterColor": "#12345"})).unwrap_err();
    assert_eq!(err.to_string(), "`display.set`: `scatterColor` must be a color ([r, g, b] linear or \"#rrggbb\")");

    let rev_before = s.revision;
    let exposure_before = s.doc.project.display.env_exposure;
    let err = execute(&mut s, "display.set", &json!({"exposure": 2.0, "scatterColor": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`display.set`: `scatterColor` must be a color ([r, g, b] linear or \"#rrggbb\")");
    assert_eq!(s.doc.project.display.env_exposure, exposure_before);
    assert_eq!(s.revision, rev_before);

    let mask_before = s.tool.mask_value;
    let err = execute(&mut s, "tool.material", &json!({"maskValue": 0.25, "channel": "baseColor", "color": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.material`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");
    assert_eq!(s.tool.mask_value, mask_before);
    assert_eq!(s.revision, rev_before);

    let err = execute(&mut s, "tool.material", &json!({"color": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.material`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");
    assert_eq!(s.revision, rev_before);

    let err = execute(&mut s, "layer.add.fill", &json!({"color": [1e40, 0.0, 0.0]})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.add.fill`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");
    assert_eq!(s.revision, rev_before);

    let err = execute(&mut s, "layer.fill.set", &json!({"channel": "baseColor", "procedural": "noise", "color": "red"})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.fill.set`: `color` must be a color ([r, g, b] linear or \"#rrggbb\")");
    assert_eq!(s.revision, rev_before);

    assert!(execute(&mut s, "tool.cloneSource", &json!({"uv": [0.2, 0.8, 0.4]})).is_err());
    execute(&mut s, "tool.polygonFill", &Value::Null).unwrap();
    let err = execute(&mut s, "tool.click", &json!({"uv": [0.2, 0.8, 0.4]})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.click`: a 2D view point needs `uv` as [u, v]");
}
