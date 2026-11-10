//! `file.importMaterialX`: a .mtlx material becomes one fill layer with its textures.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerKind, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    Session::new_project("t", primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap()
}

fn png(path: &Path, rgb: [u8; 3]) {
    image::RgbImage::from_pixel(4, 4, image::Rgb(rgb)).save(path).unwrap();
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tc_mtlx_import_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    dir
}

const MATERIAL: &str = r#"<?xml version="1.0"?>
<materialx version="1.39" fileprefix="tex/">
  <open_pbr_surface name="S" type="surfaceshader">
    <input name="base_color" type="color3" nodename="img_base" />
    <input name="specular_roughness" type="float" nodename="img_rough" />
    <input name="base_metalness" type="float" value="1" />
    <input name="geometry_normal" type="vector3" nodename="nm" />
    <input name="coat_weight" type="float" value="0.5" />
  </open_pbr_surface>
  <tiledimage name="img_base" type="color3">
    <input name="file" type="filename" value="base.png" />
    <input name="uvtiling" type="vector2" value="3, 3" />
  </tiledimage>
  <tiledimage name="img_rough" type="float"><input name="file" type="filename" value="base.png" /></tiledimage>
  <tiledimage name="img_n" type="vector3"><input name="file" type="filename" value="../../escape.png" /></tiledimage>
  <normalmap name="nm" type="vector3"><input name="in" type="vector3" nodename="img_n" /></normalmap>
  <tiledimage name="img_gone" type="float"><input name="file" type="filename" value="missing.png" /></tiledimage>
  <displacement name="D" type="displacementshader"><input name="displacement" type="float" nodename="img_gone" /></displacement>
  <surfacematerial name="Brick" type="material">
    <input name="surfaceshader" type="surfaceshader" nodename="S" />
    <input name="displacementshader" type="displacementshader" nodename="D" />
  </surfacematerial>
</materialx>"#;

fn fill_of(s: &Session, layer: u64) -> texturecraft_engine::core::Fill {
    let l = s.doc.project.layer(texturecraft_engine::core::LayerId(layer)).unwrap();
    match &l.kind {
        LayerKind::Fill(f) => f.clone(),
        other => panic!("not a fill: {other:?}"),
    }
}

#[test]
fn a_material_becomes_one_fill_layer_in_one_undo_step() {
    let dir = folder("ok");
    png(&dir.join("tex/base.png"), [200, 40, 40]);
    // A file next to the folder that a hostile document tries to reach.
    png(&dir.parent().unwrap().join("escape.png"), [1, 2, 3]);
    let mtlx = dir.join("brick.mtlx");
    std::fs::write(&mtlx, MATERIAL).unwrap();

    let mut s = session();
    let before = s.doc.resources.len();
    let r = execute(&mut s, "file.importMaterialX", &json!({ "path": mtlx.display().to_string() })).unwrap();
    assert_eq!(r["material"], "Brick");
    let fill = fill_of(&s, r["layer"].as_u64().unwrap());
    // The one image used twice is one resource; the constant is uniform.
    assert_eq!(r["resources"].as_array().unwrap().len(), 1, "{r}");
    let name = r["resources"][0].as_str().unwrap().to_string();
    assert!(matches!(fill.sources.get(&Channel::BaseColor), Some(Source::Image { name: n, .. }) if *n == name));
    assert!(matches!(fill.sources.get(&Channel::Roughness), Some(Source::Image { name: n, .. }) if *n == name));
    assert_eq!(fill.sources.get(&Channel::Metallic), Some(&Source::Uniform([1.0; 3])));
    assert!(matches!(fill.projection, texturecraft_engine::core::Projection::Uv { scale, .. } if scale == [3.0, 3.0]), "{:?}", fill.projection);
    assert_eq!(s.doc.resources.len(), before + 1);
    assert!(s.images.contains_key(&name), "the image is decoded for the viewport");

    // The traversal and the missing file were skipped with a warning, not read.
    let warnings = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert!(!fill.sources.contains_key(&Channel::Normal) && !fill.sources.contains_key(&Channel::Height), "{fill:?}");
    assert!(warnings.iter().any(|w| w.contains("outside")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("missing.png")), "{warnings:?}");
    // A channel the texture set doesn't have is reported, not invented.
    let has_coat = s.doc.project.texture_set(s.ts()).unwrap().channel_list().contains(&Channel::CoatWeight);
    assert_eq!(fill.sources.contains_key(&Channel::CoatWeight), has_coat);
    assert_eq!(warnings.iter().any(|w| w.contains("no such channel")), !has_coat, "{warnings:?}");
    assert!(!s.doc.resources.keys().any(|k| k.contains("escape")), "nothing outside the folder was imported");

    // One undo removes the layer and its images together.
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.resources.len(), before);
    assert!(s.doc.project.layer(texturecraft_engine::core::LayerId(r["layer"].as_u64().unwrap())).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(dir.parent().unwrap().join("escape.png"));
}

#[test]
fn bad_documents_and_paths_are_errors_that_change_nothing() {
    let dir = folder("bad");
    let mut s = session();
    let layers = |s: &Session| s.doc.project.texture_sets[0].stack.len();
    let (n, res) = (layers(&s), s.doc.resources.len());
    let run = |s: &mut Session, p: Value| execute(s, "file.importMaterialX", &p);
    assert!(run(&mut s, json!({})).is_err());
    assert!(run(&mut s, json!({ "path": dir.join("nope.mtlx").display().to_string() })).is_err());
    for (i, text) in ["", "garbage", "<materialx/>", "<materialx><surfacematerial name=\"M\"/></materialx>"].iter().enumerate() {
        let f = dir.join(format!("bad{i}.mtlx"));
        std::fs::write(&f, text).unwrap();
        assert!(run(&mut s, json!({ "path": f.display().to_string() })).is_err(), "{text:?}");
    }
    // Only images that can't be read: nothing to import is an error, not an empty layer.
    let only_missing = MATERIAL.replace("base.png", "none.png");
    let f = dir.join("missing.mtlx");
    std::fs::write(&f, only_missing.replace("<input name=\"base_metalness\" type=\"float\" value=\"1\" />", "")).unwrap();
    assert!(run(&mut s, json!({ "path": f.display().to_string() })).is_err());
    // A material that isn't in the document.
    std::fs::write(dir.join("m.mtlx"), MATERIAL).unwrap();
    assert!(run(&mut s, json!({ "path": dir.join("m.mtlx").display().to_string(), "material": "Nope" })).is_err());
    assert_eq!((layers(&s), s.doc.resources.len()), (n, res), "refused imports leave the project alone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_image_that_is_not_a_regular_file_is_skipped_with_a_warning() {
    let dir = folder("irregular");
    png(&dir.join("tex/base.png"), [10, 20, 30]);
    // A folder where an image should be (a pipe or device would hang the read; a folder is the portable stand-in).
    std::fs::create_dir_all(dir.join("tex/rough.png")).unwrap();
    let doc = r#"<?xml version="1.0"?>
<materialx version="1.39" fileprefix="tex/">
  <open_pbr_surface name="S" type="surfaceshader">
    <input name="base_color" type="color3" nodename="a" />
    <input name="specular_roughness" type="float" nodename="b" />
  </open_pbr_surface>
  <tiledimage name="a" type="color3"><input name="file" type="filename" value="base.png" /></tiledimage>
  <tiledimage name="b" type="float"><input name="file" type="filename" value="rough.png" /></tiledimage>
  <surfacematerial name="M" type="material"><input name="surfaceshader" type="surfaceshader" nodename="S" /></surfacematerial>
</materialx>"#;
    let mtlx = dir.join("m.mtlx");
    std::fs::write(&mtlx, doc).unwrap();
    let mut s = session();
    let r = execute(&mut s, "file.importMaterialX", &json!({ "path": mtlx.display().to_string() })).unwrap();
    assert_eq!(r["channels"], json!(["baseColor"]), "{r}");
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("regular file")), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}
