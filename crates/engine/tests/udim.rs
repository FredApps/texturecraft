//! UV tiles (UDIM): one atlas of `resolution²` blocks per texture set.

use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::geom::glam::{Vec2, Vec3};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const RES: u32 = 128;

/// The UDIM plane (tile 1001 at x < 0, 1002 at x > 0) seen from above.
fn session() -> Session {
    let mut s = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, RES).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 1.3;
    s.update();
    s
}

fn screen(s: &Session, p: Vec3) -> Vec2 {
    let n = s.camera.project(p, 1.0).unwrap();
    Vec2::new((n.x * 0.5 + 0.5) * s.viewport.x, (0.5 - n.y * 0.5) * s.viewport.y)
}

/// Red texels in the atlas block of tile 1001 (left) and 1002 (right).
fn red_per_tile(s: &Session) -> [usize; 2] {
    let c = &s.caches[0].composite;
    let mut n = [0; 2];
    for y in 0..RES {
        for x in 0..2 * RES {
            let v = c.value(Channel::BaseColor, x, y);
            if v[0] > 0.5 && v[1] < 0.2 {
                n[(x / RES) as usize] += 1;
            }
        }
    }
    n
}

fn paint_red_at(s: &mut Session, p: Vec3) {
    execute(s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(s, "tool.brush", &json!({"size": 10, "hardness": 90})).unwrap();
    let (a, b) = (screen(s, p - Vec3::X * 0.2), screen(s, p + Vec3::X * 0.2));
    execute(s, "tool.stroke", &json!({"points": [[a.x, a.y], [b.x, b.y]]})).unwrap();
    s.update();
}

#[test]
fn uv_tiles_are_detected_and_inspected() {
    let mut session = session();
    let set = execute(&mut session, "textureSet.inspect", &Value::Null).unwrap();
    assert_eq!(set["uvTiles"], json!([1001, 1002]));
    assert_eq!(set["size"], json!([2 * RES, RES]));
    assert_eq!(
        set["tiles"],
        json!([{ "udim": 1001, "block": [0, 0], "resolution": RES, "bakeEnabled": true }, { "udim": 1002, "block": [1, 0], "resolution": RES, "bakeEnabled": true }])
    );
    assert_eq!(session.caches[0].size, (2 * RES, RES));

    // Explicit modes: classic on the UDIM plane, one tile on a 0..1 mesh
    execute(&mut session, "project.new", &json!({"mesh": "udim_plane", "resolution": RES, "uvTiles": false})).unwrap();
    assert!(session.doc.project.texture_sets[0].uv_tiles.is_empty());
    execute(&mut session, "project.new", &json!({"mesh": "plane", "resolution": RES})).unwrap();
    assert!(session.doc.project.texture_sets[0].uv_tiles.is_empty(), "UVs in 0..1 stay classic");
    execute(&mut session, "project.new", &json!({"mesh": "plane", "resolution": RES, "uvTiles": true})).unwrap();
    assert_eq!(session.doc.project.texture_sets[0].uv_tiles, vec![1001]);
    assert_eq!(session.caches[0].size, (RES, RES));
}

#[test]
fn strokes_land_in_their_tile_only() {
    let mut s = session();
    let hit = s.pick(screen(&s, Vec3::new(1.0, 0.0, 0.0)), s.viewport).unwrap();
    assert!(hit.uv.x > 0.5 && hit.uv.x < 1.0, "atlas UV of tile 1002: {:?}", hit.uv);
    paint_red_at(&mut s, Vec3::new(1.0, 0.0, 0.0));
    let [left, right] = red_per_tile(&s);
    assert!(right > 100 && left == 0, "tile 1001: {left}, tile 1002: {right}");
    paint_red_at(&mut s, Vec3::new(-1.0, 0.0, 0.0));
    let [left, right2] = red_per_tile(&s);
    assert!(left > 100 && right2 == right, "tile 1001: {left}, tile 1002: {right2}");
}

#[test]
fn export_writes_one_file_per_tile() {
    let mut s = session();
    paint_red_at(&mut s, Vec3::new(1.0, 0.0, 0.0));
    let dir = std::env::temp_dir().join(format!("tc_udim_export_{}", std::process::id()));
    let r = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": "Document channels + Normal + AO"})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    let base = |tile: &str| {
        files.iter().find(|f| f.contains("ase") && f.ends_with(&format!(".{tile}.png"))).cloned().unwrap_or_else(|| panic!("no {tile} base color in {files:?}"))
    };
    let red = |path: String| {
        let img = image::open(path).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (RES, RES));
        img.pixels().filter(|p| p[0] > 200 && p[1] < 60).count()
    };
    assert_eq!(red(base("1001")), 0, "tile 1001 is unpainted");
    assert!(red(base("1002")) > 100, "tile 1002 holds the stroke");
    let plan = execute(&mut s, "export.plan", &json!({"preset": "Document channels + Normal + AO"})).unwrap();
    assert!(plan.as_array().unwrap().iter().all(|f| f["file"].as_str().unwrap().contains(".100")), "{plan}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn projects_round_trip_and_meshes_export_their_own_uvs() {
    let mut s = session();
    paint_red_at(&mut s, Vec3::new(1.0, 0.0, 0.0));
    let red = red_per_tile(&s);
    let mut back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    back.update();
    assert_eq!(back.doc.project.texture_sets[0].uv_tiles, vec![1001, 1002]);
    assert_eq!(red_per_tile(&back), red);
    // OBJ export keeps the mesh's UDIM UVs; the working mesh is in atlas space.
    assert!(s.mesh.uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x)));
    let obj = String::from_utf8(s.mesh_obj_bytes()).unwrap();
    let max_u = obj.lines().filter_map(|l| l.strip_prefix("vt ")).filter_map(|l| l.split_whitespace().next()?.parse::<f32>().ok()).fold(0.0f32, f32::max);
    assert!((max_u - 2.0).abs() < 1e-4, "{max_u}");
    // With its MTL: the same UVs, and no maps for the UV-tiled set (one image cannot hold them).
    let (with_mtl, files) = s.mesh_obj_with_mtl("plane").unwrap();
    assert!(String::from_utf8(with_mtl).unwrap().ends_with(obj.split_once("\no ").unwrap().1));
    assert_eq!(files.len(), 1, "the MTL alone");
}

#[test]
fn baking_covers_every_tile() {
    let mut s = session();
    execute(&mut s, "bake.run", &json!({"resolution": 128, "rays": 4, "maps": ["position", "ambientOcclusion"]})).unwrap();
    let baked = s.baked(0).unwrap();
    assert_eq!((baked.resolution, baked.height(BakedMap::Position)), (256, 128));
    // Position x grows across the atlas: tile 1001 holds x < 0, tile 1002 x > 0.
    let pos = &baked.maps[&BakedMap::Position];
    let x_at = |x: usize| pos[(64 * 256 + x) * 3];
    assert!(x_at(10) < 0.3 && x_at(250) > 0.7, "{} {}", x_at(10), x_at(250));
    s.update();
}

/// A UV-tiled set takes a mesh map from one image per tile, found by the tile number in the
/// name; each tile's block of the atlas reads its own image.
#[test]
fn mesh_maps_from_images_take_one_image_per_tile() {
    let mut s = session();
    let png = |v: u8| {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([v, v, v, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let set = |s: &mut Session, image: &str| execute(s, "textureSet.meshMap.set", &json!({"map": "ambientOcclusion", "image": image}));
    let left = s.import_resource_bytes("ao.1001.png", png(255)).unwrap();
    assert!(set(&mut s, &left).unwrap_err().to_string().contains("ao.1002.png"), "tile 1002 has no image yet");
    let plain = s.import_resource_bytes("ao.png", png(255)).unwrap();
    assert!(set(&mut s, &plain).is_err(), "no tile number in the name");
    s.import_resource_bytes("ao.1002.png", png(0)).unwrap();
    let r = set(&mut s, &left).unwrap()["resolution"].as_u64().unwrap() as usize;
    let ao = &s.doc.baked[&0].maps[&BakedMap::AmbientOcclusion];
    let h = ao.len() / r;
    assert_eq!(r, 2 * h, "a block of the bake size per tile, side by side");
    let at = |x: usize| ao[(h / 2) * r + x];
    assert!(at(r / 4) > 0.95 && at(3 * r / 4) < 0.05, "1001 on the left, 1002 on the right: {} {}", at(r / 4), at(3 * r / 4));
}
