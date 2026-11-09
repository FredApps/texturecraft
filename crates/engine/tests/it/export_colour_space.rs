//! `file.exportTextures` / `export.plan` with `colorSpace`.

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::texel::ColorSpace;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap();
    execute(&mut s, "layer.add.fill", &json!({ "color": [0.8, 0.2, 0.1] })).unwrap();
    s
}

fn export(s: &mut Session, tag: &str, extra: Value) -> (Value, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("tc_export_cs_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut p = json!({ "dir": dir.display().to_string(), "preset": "glTF PBR Metal Roughness", "format": "exr", "bitDepth": 32, "padding": "infinite" });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    (execute(s, "file.exportTextures", &p).unwrap(), dir)
}

fn first_pixel(path: &std::path::Path) -> [f32; 3] {
    let img = image::open(path).unwrap().to_rgb32f();
    // The plane's UV layout covers the whole image; read its middle.
    let px = img.get_pixel(img.width() / 2, img.height() / 2);
    [px[0], px[1], px[2]]
}

#[test]
fn colour_maps_are_written_in_the_chosen_space() {
    let mut s = session();
    let (plain, dir_plain) = export(&mut s, "plain", json!({}));
    let (aces, dir_aces) = export(&mut s, "aces", json!({ "colorSpace": "ACEScg" }));
    let base =
        |r: &Value| std::path::PathBuf::from(r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).find(|f| f.contains("BaseColor")).unwrap());
    let (a, b) = (first_pixel(&base(&plain)), first_pixel(&base(&aces)));
    let want = ColorSpace::AcesCg.from_linear_rec709(a);
    assert!((0..3).all(|k| (b[k] - want[k]).abs() < 1e-3), "{b:?} vs {want:?}");
    assert!((0..3).any(|k| (a[k] - b[k]).abs() > 0.01), "the colour actually changed: {a:?} vs {b:?}");
    for d in [dir_plain, dir_aces] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn the_plan_reports_the_space_and_clamping_and_bad_names_are_errors() {
    let mut s = session();
    let plan = execute(&mut s, "export.plan", &json!({ "preset": "glTF PBR Metal Roughness", "colorSpace": "ACEScg" })).unwrap();
    assert!(plan.as_array().unwrap().iter().any(|f| f["warnings"].as_array().is_some_and(|w| !w.is_empty())), "8-bit ACEScg warns: {plan}");
    let float = execute(&mut s, "export.plan", &json!({ "preset": "glTF PBR Metal Roughness", "colorSpace": "ACEScg", "format": "exr" })).unwrap();
    assert!(float.as_array().unwrap().iter().all(|f| f["warnings"].as_array().is_some_and(Vec::is_empty)), "{float}");
    for bad in ["Display P3", "", "Rec.2020"] {
        assert!(execute(&mut s, "export.plan", &json!({ "preset": "glTF PBR Metal Roughness", "colorSpace": bad })).is_err(), "{bad:?}");
    }
    // A value that isn't a name is an error too, not the format's default space; null means none.
    for bad in [json!(["ACEScg"]), json!(3), json!({ "name": "ACEScg" }), json!(true)] {
        assert!(execute(&mut s, "export.plan", &json!({ "preset": "glTF PBR Metal Roughness", "colorSpace": bad })).is_err(), "{bad}");
    }
    assert!(execute(&mut s, "export.plan", &json!({ "preset": "glTF PBR Metal Roughness", "colorSpace": null })).is_ok());
}

fn png_icc(path: &str) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    image::codecs::png::PngDecoder::new(file).unwrap().icc_profile().unwrap()
}

fn find<'a>(r: &'a Value, part: &str) -> &'a str {
    r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).find(|f| f.contains(part)).unwrap_or_else(|| panic!("no {part} in {r}"))
}

#[test]
fn a_chosen_colour_space_is_recorded_in_colour_maps_only() {
    let mut s = session();
    let png = |extra: Value| {
        let mut p = json!({ "format": "png", "bitDepth": 8 });
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let (default, d0) = export(&mut s, "icc_default", png(json!({})));
    assert_eq!(png_icc(find(&default, "BaseColor")), None, "nothing is written unless a space is chosen");
    let (aces, d1) = export(&mut s, "icc_aces", png(json!({ "colorSpace": "ACEScg" })));
    let icc = png_icc(find(&aces, "BaseColor")).expect("the base colour map carries a profile");
    assert!(icc.windows(6).any(|w| w == b"ACEScg"), "the profile names its space");
    assert_eq!(Some(icc), texturecraft_formats::icc::profile(ColorSpace::AcesCg));
    assert_eq!(png_icc(find(&aces, "Normal")), None, "a normal map is data and has no colour space");
    // Formats that can't hold one say so, once per colour map.
    let (tga, d2) = export(&mut s, "icc_tga", png(json!({ "colorSpace": "ACEScg", "format": "tga" })));
    assert!(tga["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("TGA") && w.as_str().unwrap().contains("colour space")), "{tga}");
    for d in [d0, d1, d2] {
        let _ = std::fs::remove_dir_all(d);
    }
}
