use image::{DynamicImage, GenericImageView, ImageFormat};
use texturecraft_core::{ChannelFormat, Project, TextureSet};
use texturecraft_texel::{to_u16, to_u16_signed};

use super::*;
use crate::presets::{builtin_preset, builtin_presets, preset_from_json, preset_to_json};

const RES: u32 = 64;

struct Fx {
    project: Project,
    comp: Composite,
    baked: BakedMaps,
    coverage: Vec<bool>,
}

impl Fx {
    fn new(channels: &[Channel]) -> Fx {
        let mut project = Project::new("My Project", "builtin:sphere");
        let mut ts = TextureSet::new("Body Set", 0, RES);
        ts.channels = channels.iter().map(|c| (*c, ChannelFormat::default_for(*c))).collect();
        project.texture_sets.push(ts);
        let mut fx = Fx { project, comp: Composite::new(channels, RES), baked: BakedMaps::default(), coverage: vec![true; (RES * RES) as usize] };
        for y in 0..RES {
            for x in 0..RES {
                fx.set(Channel::BaseColor, x, y, [x as f32 / 63.0, y as f32 / 63.0, 0.2]);
                fx.set(Channel::Roughness, x, y, [0.25; 3]);
                fx.set(Channel::Metallic, x, y, [0.75; 3]);
                fx.set(Channel::Normal, x, y, [0.5, 0.5, 1.0]);
                fx.set(Channel::Height, x, y, [0.0; 3]);
            }
        }
        fx
    }

    fn set(&mut self, c: Channel, x: u32, y: u32, v: [f32; 3]) {
        let Some(img) = self.comp.channels.get_mut(&c) else { return };
        let vals: Vec<u16> = v.iter().take(c.components()).map(|v| if c.kind() == ChannelKind::Signed { to_u16_signed(*v) } else { to_u16(*v) }).collect();
        img.set(x, y, &vals);
    }

    fn set_all(&mut self, c: Channel, f: impl Fn(u32, u32) -> [f32; 3]) {
        for y in 0..RES {
            for x in 0..RES {
                self.set(c, x, y, f(x, y));
            }
        }
    }

    fn input(&self) -> ExportInput<'_> {
        ExportInput { project: &self.project, ts: 0, composite: &self.comp, baked: &self.baked, coverage: &self.coverage, mesh_name: "sphere" }
    }

    fn render(&self, opts: &ExportOptions) -> Vec<ExportedFile> {
        render_maps(&self.input(), opts).unwrap()
    }

    fn render_preset(&self, name: &str) -> Vec<ExportedFile> {
        self.render(&ExportOptions::new(builtin_preset(name).unwrap()))
    }
}

fn find<'a>(files: &'a [ExportedFile], suffix: &str) -> &'a ExportedFile {
    files.iter().find(|f| f.name.contains(suffix)).unwrap_or_else(|| panic!("no {suffix} in {:?}", files.iter().map(|f| &f.name).collect::<Vec<_>>()))
}

fn decode(f: &ExportedFile) -> DynamicImage {
    let ext = f.name.rsplit('.').next().unwrap();
    image::load_from_memory_with_format(&f.bytes, ImageFormat::from_extension(ext).unwrap()).unwrap()
}

fn default_channels() -> Vec<Channel> {
    Channel::DEFAULT_SET.to_vec()
}

#[test]
fn gltf_png_matches_the_composite() {
    let fx = Fx::new(&default_channels());
    let files = fx.render_preset("glTF PBR Metal Roughness");
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["sphere_Body_Set_BaseColor.png", "sphere_Body_Set_OcclusionRoughnessMetallic.png", "sphere_Body_Set_Normal.png"]);
    let base = decode(&files[0]);
    assert_eq!(base.color(), image::ColorType::Rgb8, "no opacity channel → no alpha");
    for (x, y) in [(0, 0), (10, 40), (63, 63), (32, 7)] {
        let v = fx.comp.value(Channel::BaseColor, x, y);
        let px = base.get_pixel(x, y);
        for k in 0..3 {
            let want = linear_to_srgb(v[k]) * 255.0;
            assert!((px[k] as f32 - want).abs() <= 1.0, "base ({x},{y})[{k}] {} vs {want}", px[k]);
        }
    }
    let orm = decode(&files[1]).get_pixel(5, 5);
    assert_eq!(orm[0], 255, "no AO anywhere → white");
    assert!((orm[1] as i32 - 64).abs() <= 1 && (orm[2] as i32 - 191).abs() <= 1, "{orm:?}");
    let n = decode(&files[2]).get_pixel(5, 5);
    assert!((n[0] as i32 - 128).abs() <= 1 && (n[1] as i32 - 128).abs() <= 1 && n[2] == 255, "{n:?}");
    // Emissive appears only when the set has the channel.
    let mut channels = default_channels();
    channels.push(Channel::Emissive);
    let fx = Fx::new(&channels);
    assert!(fx.render_preset("glTF PBR Metal Roughness").iter().any(|f| f.name.ends_with("_Emissive.png")));
}

#[test]
fn exr_values_are_linear() {
    let fx = Fx::new(&default_channels());
    for (depth, tol) in [(32u8, 1e-4f32), (16, 2e-3)] {
        let mut opts = ExportOptions::new(builtin_preset("glTF PBR Metal Roughness").unwrap());
        opts.format_override = Some(FileFormat::Exr);
        opts.bit_depth_override = Some(depth);
        let files = fx.render(&opts);
        let base = find(&files, "_BaseColor.exr");
        let img = decode(base).to_rgb32f();
        for (x, y) in [(0, 0), (10, 40), (63, 63)] {
            let v = fx.comp.value(Channel::BaseColor, x, y);
            let px = img.get_pixel(x, y);
            for k in 0..3 {
                assert!((px[k] - v[k]).abs() <= tol.max(v[k] * tol), "depth {depth} ({x},{y})[{k}]: {} vs {}", px[k], v[k]);
            }
        }
    }
    // A single-channel map is written as a luminance (Y) EXR.
    let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
    opts.format_override = Some(FileFormat::Exr);
    let files = fx.render(&opts);
    let rough = find(&files, "_roughness.exr");
    use exr::prelude::{ReadChannels, ReadLayers};
    let img = exr::prelude::read()
        .no_deep_data()
        .largest_resolution_level()
        .all_channels()
        .first_valid_layer()
        .all_attributes()
        .from_buffered(std::io::Cursor::new(&rough.bytes))
        .unwrap();
    let ch = &img.layer_data.channel_data.list;
    assert_eq!(ch.len(), 1);
    assert_eq!(ch[0].name.to_string(), "Y");
    assert!((ch[0].sample_data.value_by_flat_index(100).to_f32() - 0.25).abs() < 1e-3);
}

#[test]
fn directx_flips_green() {
    let mut fx = Fx::new(&default_channels());
    let n = Vec3::new(0.3, 0.4, 0.0);
    let n = Vec3::new(n.x, n.y, (1.0 - n.length_squared()).sqrt());
    fx.set_all(Channel::Normal, |_, _| (n * 0.5 + Vec3::splat(0.5)).to_array());
    let gl = decode(find(&fx.render_preset("glTF PBR Metal Roughness"), "_Normal.png")).to_rgb8();
    let dx = decode(find(&fx.render_preset("Unreal Engine (Packed)"), "_Normal.png")).to_rgb8();
    let (a, b) = (gl.get_pixel(9, 9), dx.get_pixel(9, 9));
    assert!((a[0] as i32 - ((n.x * 0.5 + 0.5) * 255.0).round() as i32).abs() <= 1);
    assert!((a[1] as i32 - ((n.y * 0.5 + 0.5) * 255.0).round() as i32).abs() <= 1, "{a:?}");
    assert_eq!(a[0], b[0]);
    assert!((a[1] as i32 + b[1] as i32 - 255).abs() <= 1, "{a:?} vs {b:?}");
    assert_eq!(a[2], b[2]);
    // The normal-format override applies to every normal output.
    let mut opts = ExportOptions::new(builtin_preset("glTF PBR Metal Roughness").unwrap());
    opts.normal_format = Some(NormalFormat::DirectX);
    assert_eq!(decode(find(&fx.render(&opts), "_Normal.png")).to_rgb8(), dx);
}

#[test]
fn height_becomes_normal_detail() {
    let mut fx = Fx::new(&[Channel::BaseColor, Channel::Height]);
    // Height rises to the right and down the image.
    fx.set_all(Channel::Height, |x, y| [(x as f32 + y as f32) / 160.0; 3]);
    fx.project.texture_sets[0].height_strength = 8.0;
    let files = fx.render_preset("Separate maps");
    let n = decode(find(&files, "_normal.png")).to_rgb8();
    let p = n.get_pixel(30, 30);
    assert!(p[0] < 120, "slope up to the right tilts the normal left: {p:?}");
    assert!(p[1] > 136, "slope up toward the image bottom tilts an OpenGL normal up: {p:?}");
    let h = decode(find(&files, "_height.png"));
    assert_eq!(h.color(), image::ColorType::L16);
    let v = h.to_luma16().get_pixel(30, 30)[0] as f32 / 65535.0;
    assert!((v - (60.0 / 160.0 * 0.5 + 0.5)).abs() < 1e-3, "height is stored as h·0.5+0.5: {v}");
    fx.project.texture_sets[0].height_strength = 0.0;
    let flat = decode(find(&fx.render_preset("Separate maps"), "_normal.png")).to_rgb8();
    assert_eq!(flat.get_pixel(30, 30).0, [128, 128, 255]);
}

#[test]
fn reoriented_normal_mapping_identities() {
    let d = Vec3::new(0.2, -0.3, 0.9).normalize();
    assert!((rnm(Vec3::Z, d) - d).length() < 1e-5);
    assert!((rnm(d, Vec3::Z) - d).length() < 1e-5);
    assert!(rnm(Vec3::new(0.0, 0.0, -1.0), d).is_finite());
}

#[test]
fn ao_mixes_baked_and_channel() {
    let mut channels = default_channels();
    channels.push(Channel::AmbientOcclusion);
    let mut fx = Fx::new(&channels);
    fx.set_all(Channel::AmbientOcclusion, |_, _| [0.5; 3]);
    fx.baked = BakedMaps { resolution: RES, maps: [(BakedMap::AmbientOcclusion, std::sync::Arc::new(vec![0.5; (RES * RES) as usize]))].into() };
    fx.project.texture_sets[0].baked.insert(BakedMap::AmbientOcclusion);
    let orm = decode(find(&fx.render_preset("glTF PBR Metal Roughness"), "Metallic.png")).to_rgb8();
    assert!((orm.get_pixel(3, 3)[0] as i32 - 64).abs() <= 1);
    let mesh_maps = fx.render_preset("Mesh maps");
    assert_eq!(mesh_maps.len(), 1, "only baked maps are exported");
    let ao = decode(&mesh_maps[0]);
    assert_eq!(mesh_maps[0].name, "sphere_Body_Set_ambientOcclusion.png");
    assert_eq!(ao.color(), image::ColorType::L16);
}

#[test]
fn templates_are_sanitized_and_collisions_suffixed() {
    let fx = Fx::new(&default_channels());
    let rough = |file: &str| MapSpec {
        file: file.into(),
        format: FileFormat::Png,
        bit_depth: 8,
        channels: vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 })],
    };
    let preset = ExportPreset {
        name: "t".into(),
        maps: vec![
            rough("$project $textureSet:$channel ($colorSpace)"),
            rough("$project $textureSet:$channel ($colorSpace)"),
            rough("con"),
            rough("A"),
            rough("a"),
            rough("."),
        ],
    };
    let opts = ExportOptions::new(preset);
    let planned = list_outputs(&fx.project, 0, "sphere", &opts).unwrap();
    let names: Vec<&str> = planned.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["My_Project_Body_Set_roughness__linear_.png", "My_Project_Body_Set_roughness__linear__2.png", "_con.png", "A.png", "a_2.png", "texture.png"]
    );
    assert!(planned[0].warnings.is_empty());
    assert_eq!(planned[1].warnings.len(), 1);
    assert_eq!(planned[4].warnings.len(), 1);
    let rendered: Vec<String> = fx.render(&opts).into_iter().map(|f| f.name).collect();
    assert_eq!(rendered, names);
}

#[test]
fn padding_modes() {
    let mut fx = Fx::new(&default_channels());
    fx.coverage = (0..RES * RES).map(|i| i % RES < 32).collect();
    let rough_at = |fx: &Fx, padding: Padding, x: u32| {
        let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
        opts.padding = padding;
        decode(find(&fx.render(&opts), "_roughness.png")).to_luma8().get_pixel(x, 10)[0]
    };
    assert_eq!(rough_at(&fx, Padding::None, 10), 64);
    assert_eq!(rough_at(&fx, Padding::None, 40), 0);
    assert_eq!(rough_at(&fx, Padding::Infinite, 63), 64);
    assert_eq!(rough_at(&fx, Padding::Dilation(2), 33), 64);
    assert_eq!(rough_at(&fx, Padding::Dilation(2), 40), 0);
    assert_eq!(rough_at(&fx, Padding::DilationDefault(2), 33), 64);
    assert_eq!(rough_at(&fx, Padding::DilationDefault(2), 40), 128, "roughness default is 0.5");
    let bad = Fx { coverage: vec![true; 10], ..Fx::new(&default_channels()) };
    assert!(render_maps(&bad.input(), &ExportOptions::new(builtin_preset("Separate maps").unwrap())).is_err());
}

#[test]
fn output_size_override() {
    let fx = Fx::new(&default_channels());
    for size in [32u32, 128] {
        let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
        opts.size = Some(size);
        let files = fx.render(&opts);
        let r = find(&files, "_roughness.png");
        assert_eq!((r.width, r.height), (size, size));
        let img = decode(r).to_luma8();
        assert_eq!(img.dimensions(), (size, size));
        assert_eq!(img.get_pixel(size / 2, size / 3)[0], 64);
    }
    let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
    opts.size = Some(48);
    assert!(render_maps(&fx.input(), &opts).is_err());
}

#[test]
fn every_format_encodes_and_decodes() {
    let mut channels = default_channels();
    channels.push(Channel::Opacity);
    let fx = Fx::new(&channels);
    let gray_alpha = MapSpec {
        file: "la".into(),
        format: FileFormat::Png,
        bit_depth: 8,
        channels: vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 }), (Dest::A, SourceRef::Constant(0.5))],
    };
    let mut preset = builtin_preset("glTF PBR Metal Roughness").unwrap();
    preset.maps.push(gray_alpha);
    for format in FileFormat::ALL {
        for &depth in format.bit_depths() {
            let mut opts = ExportOptions::new(preset.clone());
            opts.format_override = Some(format);
            opts.bit_depth_override = Some(depth);
            let files = fx.render(&opts);
            assert_eq!(files.len(), 4, "{format:?}");
            for f in &files {
                assert!(f.name.ends_with(format.extension()));
                if format == FileFormat::Exr && f.name.starts_with("la") {
                    continue; // Y/A EXR: covered by `exr_values_are_linear`.
                }
                assert_eq!(decode(f).dimensions(), (RES, RES), "{}", f.name);
            }
            // Roughness 0.25 everywhere survives every format (JPEG within its loss).
            let px = decode(find(&files, "OcclusionRoughnessMetallic.")).to_rgba32f().get_pixel(1, 1)[1];
            assert!((px - 0.25).abs() < 0.01, "{format:?}/{depth}: {px}");
            let base = find(&files, "_BaseColor.");
            match format {
                FileFormat::Jpeg | FileFormat::Hdr => assert!(base.warnings.iter().any(|w| w.contains("alpha")), "{format:?}"),
                _ => assert!(decode(base).color().has_alpha(), "{format:?} keeps opacity"),
            }
        }
    }
}

#[test]
fn list_outputs_matches_render_and_presets_round_trip() {
    let fx = Fx::new(&default_channels());
    for preset in builtin_presets() {
        preset.validate().unwrap();
        assert_eq!(preset_from_json(&preset_to_json(&preset).unwrap()).unwrap(), preset);
        let opts = ExportOptions::new(preset.clone());
        let planned: Vec<String> = list_outputs(&fx.project, 0, "sphere", &opts).unwrap().into_iter().map(|p| p.name).collect();
        let rendered: Vec<String> = fx.render(&opts).into_iter().map(|f| f.name).collect();
        assert_eq!(planned, rendered, "{}", preset.name);
    }
    assert!(builtin_preset("unity hdrp (mask map)").is_some());
    // OpenPBR writes the lobe maps a set has, and skips the rest.
    let openpbr = ExportOptions::new(builtin_preset("OpenPBR (separate maps)").unwrap());
    let names = |fx: &Fx| list_outputs(&fx.project, 0, "m", &openpbr).unwrap().into_iter().map(|p| p.name).collect::<Vec<String>>();
    let plain = names(&fx);
    assert!(plain.iter().any(|n| n.ends_with("_base_color.png")) && !plain.iter().any(|n| n.contains("coat_")), "{plain:?}");
    let mut lobes = default_channels();
    lobes.extend([Channel::CoatWeight, Channel::CoatRoughness, Channel::SheenWeight]);
    let coated = names(&Fx::new(&lobes));
    assert!(coated.iter().any(|n| n.ends_with("_coat_weight.png")) && coated.iter().any(|n| n.ends_with("_fuzz_weight.png")), "{coated:?}");
    assert!(builtin_preset("godot 4 (orm)").is_some());
    assert!(preset_from_json("{").is_err());
    let dup = r#"{"name":"x","maps":[{"file":"a","format":"Png","bit_depth":8,"channels":[["R",{"Constant":1.0}],["R",{"Constant":0.0}]]}]}"#;
    assert!(preset_from_json(dup).is_err());
    let mixed = r#"{"name":"x","maps":[{"file":"a","format":"Png","bit_depth":8,"channels":[["L",{"Constant":1.0}],["G",{"Constant":0.0}]]}]}"#;
    assert!(preset_from_json(mixed).is_err());
    let ok = r#"{"name":"x","maps":[{"file":"a","format":"Tga","bit_depth":8,"channels":[["L",{"Channel":{"channel":"Metallic"}}]]}]}"#;
    assert!(preset_from_json(ok).is_ok());
    assert!(list_outputs(&fx.project, 5, "m", &ExportOptions::new(builtin_presets().remove(0))).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn write_files_is_atomic_and_refuses_paths() {
    let fx = Fx::new(&default_channels());
    let files = fx.render_preset("Unreal Engine (Packed)");
    let dir = std::env::temp_dir().join(format!("tc-formats-export-{}", std::process::id()));
    let paths = write_files(&files, &dir).unwrap();
    assert_eq!(paths.len(), 3);
    for (p, f) in paths.iter().zip(&files) {
        assert_eq!(std::fs::read(p).unwrap(), f.bytes);
    }
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    let evil = ExportedFile { name: "../evil.png".into(), ..files[0].clone() };
    assert!(write_files(&[evil], &dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn write_files_accepts_planned_name_with_max_length_stem() {
    let fx = Fx::new(&default_channels());
    let files = fx.render_preset("Unreal Engine (Packed)");
    let name = format!("{}.png", sanitize_file_stem(&"a".repeat(250)));
    let dir = std::env::temp_dir().join(format!("tc-formats-export-long-{}", std::process::id()));
    let long = ExportedFile { name, ..files[0].clone() };
    let paths = write_files(&[long], &dir).unwrap();
    assert_eq!(paths.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn gray_map(file: &str, src: SourceRef) -> MapSpec {
    MapSpec { file: file.into(), format: FileFormat::Png, bit_depth: 8, channels: vec![(Dest::L, src)] }
}

#[test]
fn validation_rejects_bad_presets() {
    let ok = ExportPreset { name: "Mine".into(), maps: vec![gray_map("$mesh_$textureSet_$channel ($colorSpace) $project", SourceRef::Constant(1.0))] };
    ok.validate().unwrap();
    let refused = |f: &dyn Fn(&mut ExportPreset)| {
        let mut p = ok.clone();
        f(&mut p);
        p.validate().is_err()
    };
    let long = "x".repeat(300);
    for file in ["a/b", r"a\b", "..", "x..y", "$meshes_$foo", "$", "", " ", "a\u{7}b", long.as_str()] {
        assert!(refused(&|p| p.maps[0].file = file.into()), "file template {file:?} must be refused");
    }
    assert!(refused(&|p| p.name = "  ".into()));
    assert!(refused(&|p| p.name = "x".repeat(MAX_PRESET_NAME + 1)));
    assert!(refused(&|p| p.name = "a\nb".into()));
    assert!(refused(&|p| p.maps[0].bit_depth = 12));
    assert!(refused(&|p| p.maps[0].channels[0].1 = SourceRef::Channel { channel: Channel::BaseColor, component: 3 }));
    assert!(refused(&|p| p.maps[0].channels[0].1 = SourceRef::Constant(f32::NAN)));
    assert!(refused(&|p| p.maps = vec![p.maps[0].clone(); MAX_MAPS + 1]));
    // The same checks guard JSON from disk or agents.
    let json = preset_to_json(&ok).unwrap();
    assert_eq!(preset_from_json(&json).unwrap(), ok);
    assert!(preset_from_json(&json.replace("$mesh_", "../")).is_err());
    assert!(preset_from_json(&" ".repeat(2 << 20)).is_err());
}

#[test]
fn converted_maps_invert_and_derive_normals() {
    let mut fx = Fx::new(&[Channel::BaseColor, Channel::Roughness, Channel::Glossiness, Channel::Height]);
    fx.set_all(Channel::Glossiness, |_, _| [0.2; 3]);
    fx.set_all(Channel::Height, |x, y| [(x as f32 + y as f32) / 160.0; 3]);
    fx.project.texture_sets[0].height_strength = 8.0;
    let virt = |map| SourceRef::Virtual { map, component: 0 };
    let rgb = |file: &str, map| MapSpec {
        file: file.into(),
        format: FileFormat::Png,
        bit_depth: 8,
        channels: [Dest::R, Dest::G, Dest::B].into_iter().zip(0u8..).map(|(d, k)| (d, SourceRef::Virtual { map, component: k })).collect(),
    };
    let preset = ExportPreset {
        name: "converted".into(),
        maps: vec![
            gray_map("gloss", virt(VirtualMap::Glossiness)),
            gray_map("rough", virt(VirtualMap::Roughness)),
            rgb("hn", VirtualMap::HeightNormal),
            rgb("dx", VirtualMap::NormalDirectX),
        ],
    };
    let files = fx.render(&ExportOptions::new(preset));
    assert_eq!(decode(find(&files, "gloss")).to_luma8().get_pixel(4, 4)[0], 191, "1 - roughness 0.25");
    assert_eq!(decode(find(&files, "rough")).to_luma8().get_pixel(4, 4)[0], 204, "1 - glossiness 0.2");
    let hn = decode(find(&files, "hn")).to_rgb8();
    let full = decode(find(&fx.render_preset("Separate maps"), "_normal.png")).to_rgb8();
    assert_eq!(hn.get_pixel(30, 30), full.get_pixel(30, 30), "without a normal channel the final normal is the height normal");
    let (a, b) = (hn.get_pixel(30, 30), decode(find(&files, "dx")).to_rgb8().get_pixel(30, 30).to_owned());
    assert!(a[1] > 136 && (a[1] as i32 + b[1] as i32 - 255).abs() <= 1, "DirectX flips green: {a:?} vs {b:?}");
    // Converted maps only exist when their source channel does.
    let fx = Fx::new(&default_channels());
    let only = ExportPreset { name: "c".into(), maps: vec![gray_map("rough", virt(VirtualMap::Roughness)), rgb("hn", VirtualMap::HeightNormal)] };
    let names: Vec<String> = list_outputs(&fx.project, 0, "m", &ExportOptions::new(only)).unwrap().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["hn.png"], "the default set has height but no glossiness");
}

#[test]
fn uv_tiles_export_one_file_per_tile_without_padding_across() {
    // Tiles 1001 (left block: red, only x < 16 covered) and 1002 (right block: blue).
    let mut project = Project::new("My Project", "builtin:udim_plane");
    let mut ts = TextureSet::new("Body Set", 0, RES);
    ts.uv_tiles = vec![1001, 1002];
    ts.channels = vec![(Channel::BaseColor, ChannelFormat::default_for(Channel::BaseColor))];
    project.texture_sets.push(ts);
    let mut comp = Composite::with_size(&[Channel::BaseColor], 2 * RES, RES);
    let mut coverage = vec![false; (2 * RES * RES) as usize];
    for y in 0..RES {
        for x in 0..2 * RES {
            let blue = x >= RES;
            if blue || x < 16 {
                comp.channels.get_mut(&Channel::BaseColor).unwrap().set(x, y, &[to_u16(if blue { 0.0 } else { 1.0 }), 0, to_u16(if blue { 1.0 } else { 0.0 })]);
                coverage[(y * 2 * RES + x) as usize] = true;
            }
        }
    }
    let input = ExportInput { project: &project, ts: 0, composite: &comp, baked: &BakedMaps::default(), coverage: &coverage, mesh_name: "udim_plane" };
    let preset = ExportPreset {
        name: "t".into(),
        maps: vec![MapSpec {
            file: "$textureSet_color".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![
                (Dest::R, SourceRef::Channel { channel: Channel::BaseColor, component: 0 }),
                (Dest::G, SourceRef::Channel { channel: Channel::BaseColor, component: 1 }),
                (Dest::B, SourceRef::Channel { channel: Channel::BaseColor, component: 2 }),
            ],
        }],
    };
    let files = render_maps(&input, &ExportOptions::new(preset.clone())).unwrap();
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Body_Set_color.1001.png", "Body_Set_color.1002.png"]);
    let (left, right) = (decode(&files[0]).to_rgb8(), decode(&files[1]).to_rgb8());
    assert_eq!((left.width(), left.height(), right.width()), (RES, RES, RES));
    // Infinite padding fills tile 1001 from its own island only.
    assert!(left.pixels().all(|p| p.0 == [255, 0, 0]), "tile 1001 is red");
    assert!(right.pixels().all(|p| p.0 == [0, 0, 255]), "tile 1002 is blue");
    // `$udim` placed by the template; a size applies per tile; the atlas is one image.
    let mut named = preset.clone();
    named.maps[0].file = "$udim_$textureSet".into();
    let opts = ExportOptions { size: Some(32), ..ExportOptions::new(named) };
    let small = render_maps(&input, &opts).unwrap();
    assert_eq!((small[1].name.as_str(), small[1].width, small[1].height), ("1002_Body_Set.png", 32, 32));
    let atlas = render_maps(&input, &ExportOptions { atlas: true, ..ExportOptions::new(preset) }).unwrap();
    assert_eq!((atlas.len(), atlas[0].name.as_str(), atlas[0].width, atlas[0].height), (1, "Body_Set_color.png", 2 * RES, RES));
}

#[test]
fn the_udim_name_is_the_file_name_with_its_tile_number_as_placeholder() {
    let mut project = Project::new("p", "builtin:udim_plane");
    let mut ts = TextureSet::new("Body", 0, RES);
    ts.uv_tiles = vec![1001, 1002];
    ts.channels = vec![(Channel::BaseColor, ChannelFormat::default_for(Channel::BaseColor))];
    project.texture_sets.push(ts);
    let udim_names = |file: String| {
        let channels = vec![(Dest::R, SourceRef::Channel { channel: Channel::BaseColor, component: 0 })];
        let preset = ExportPreset { name: "t".into(), maps: vec![MapSpec { file, format: FileFormat::Png, bit_depth: 8, channels }] };
        list_outputs(&project, 0, "m", &ExportOptions::new(preset)).unwrap().into_iter().map(|f| f.udim_name).collect::<Vec<_>>()
    };
    let placeholder = Some("Body_<UDIM>.png".to_string());
    assert_eq!(udim_names("$textureSet_$udim".into()), [placeholder.clone(), placeholder]);
    // The length limit cuts the longer placeholder where it doesn't cut the tile number: none then.
    assert_eq!(udim_names(format!("{}_$udim_$textureSet", "a".repeat(MAX_STEM - 10))), [None, None]);
}

#[test]
fn lobe_channels_export_as_document_maps_and_gltf_extension_maps() {
    let mut channels = default_channels();
    channels.extend(Channel::LOBES);
    let mut fx = Fx::new(&channels);
    fx.set_all(Channel::CoatWeight, |_, _| [1.0; 3]);
    fx.set_all(Channel::CoatRoughness, |_, _| [0.2; 3]);
    fx.set_all(Channel::SheenWeight, |_, _| [0.5; 3]);
    fx.set_all(Channel::SheenColor, |_, _| [1.0, 0.5, 0.0]);
    fx.set_all(Channel::SheenRoughness, |_, _| [0.4; 3]);
    let doc = fx.render_preset("Document channels + Normal + AO");
    for c in Channel::LOBES {
        find(&doc, &format!("_{}.", c.id()));
    }
    let gltf = fx.render_preset("glTF PBR Metal Roughness");
    let coat = decode(find(&gltf, "_Clearcoat.png")).to_rgb8();
    let px = coat.get_pixel(3, 3);
    assert!(px[0] == 255 && (px[1] as i32 - 51).abs() <= 1 && px[2] == 0, "weight in R, roughness in G: {px:?}");
    let sheen = decode(find(&gltf, "_Sheen.png")).to_rgba8();
    let px = sheen.get_pixel(3, 3);
    // sRGB color × weight (0.5, 0.25, 0) and linear roughness in alpha.
    let want = [188, 137, 0, 102];
    assert!(px.0.iter().zip(want).all(|(a, b)| (*a as i32 - b).abs() <= 1), "{px:?}");
    let plain = Fx::new(&default_channels()).render_preset("glTF PBR Metal Roughness");
    assert!(!plain.iter().any(|f| f.name.contains("Clearcoat") || f.name.contains("Sheen")), "sets without the lobes export no extension maps");
}

#[test]
fn colour_space_converts_colour_maps_only() {
    let fx = Fx::new(&default_channels());
    let preset = builtin_preset("glTF PBR Metal Roughness").unwrap();
    let want_at = |space: ColorSpace, x, y| space.from_linear_rec709(fx.comp.value(Channel::BaseColor, x, y));
    // ACEScg in a float EXR: the base colour is the converted triple, above-1 values aside.
    let mut exr = ExportOptions::new(preset.clone());
    exr.format_override = Some(FileFormat::Exr);
    exr.bit_depth_override = Some(32);
    exr.color_space = Some(ColorSpace::AcesCg);
    let files = fx.render(&exr);
    let img = decode(find(&files, "_BaseColor.exr")).to_rgb32f();
    for (x, y) in [(0, 0), (10, 40), (63, 63)] {
        let (px, want) = (img.get_pixel(x, y), want_at(ColorSpace::AcesCg, x, y));
        assert!((0..3).all(|k| (px[k] - want[k]).abs() < 1e-4), "({x},{y}) {px:?} vs {want:?}");
    }
    // Data maps are never converted: roughness and metalness (packed in the ORM map) are as before.
    let default_files =
        fx.render(&ExportOptions { format_override: Some(FileFormat::Exr), bit_depth_override: Some(32), ..ExportOptions::new(preset.clone()) });
    assert_eq!(find(&files, "OcclusionRoughnessMetallic").bytes, find(&default_files, "OcclusionRoughnessMetallic").bytes);
    assert_eq!(find(&files, "_Normal").bytes, find(&default_files, "_Normal").bytes);
    assert_ne!(find(&files, "_BaseColor").bytes, find(&default_files, "_BaseColor").bytes);

    // Linear Rec.709 in an 8-bit PNG: the values are written without the sRGB curve.
    let mut lin = ExportOptions::new(preset.clone());
    lin.color_space = Some(ColorSpace::LinearRec709);
    let png = decode(find(&fx.render(&lin), "_BaseColor.png"));
    let v = fx.comp.value(Channel::BaseColor, 10, 40);
    for k in 0..3 {
        assert!((png.get_pixel(10, 40)[k] as f32 - v[k] * 255.0).abs() <= 1.0, "[{k}]");
    }
    // The default is unchanged: sRGB for integer files (None and explicit sRGB agree).
    let default_png = fx.render(&ExportOptions::new(preset.clone()));
    let mut srgb = ExportOptions::new(preset.clone());
    srgb.color_space = Some(ColorSpace::Srgb);
    let srgb_png = fx.render(&srgb);
    assert_eq!(decode(find(&default_png, "_BaseColor.png")).to_rgb8(), decode(find(&srgb_png, "_BaseColor.png")).to_rgb8());
    // The chosen one is written into the file as its profile; the default says nothing, as before.
    assert_eq!(embedded_icc(FileFormat::Png, &find(&srgb_png, "_BaseColor.png").bytes), crate::icc::profile(ColorSpace::Srgb));
    assert_eq!(embedded_icc(FileFormat::Png, &find(&default_png, "_BaseColor.png").bytes), None);
    // Data maps carry none, even when a space is chosen.
    assert_eq!(embedded_icc(FileFormat::Png, &find(&srgb_png, "_Normal.png").bytes), None);
    // An explicit sRGB in a float file is encoded once, not twice.
    let mut float_srgb = ExportOptions::new(preset);
    float_srgb.format_override = Some(FileFormat::Exr);
    float_srgb.bit_depth_override = Some(32);
    float_srgb.color_space = Some(ColorSpace::Srgb);
    let px = *decode(find(&fx.render(&float_srgb), "_BaseColor.exr")).to_rgb32f().get_pixel(10, 40);
    assert!((px[0] - linear_to_srgb(v[0])).abs() < 1e-4, "{px:?}");
}

#[test]
fn colour_space_names_the_files_and_warns_when_integer_files_clamp() {
    let fx = Fx::new(&default_channels());
    let named = ExportPreset {
        name: "named".into(),
        maps: vec![MapSpec {
            file: "$textureSet_$channel_$colorSpace".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![
                (Dest::R, SourceRef::Channel { channel: Channel::BaseColor, component: 0 }),
                (Dest::G, SourceRef::Channel { channel: Channel::BaseColor, component: 1 }),
                (Dest::B, SourceRef::Channel { channel: Channel::BaseColor, component: 2 }),
            ],
        }],
    };
    let plan = |space: Option<ColorSpace>, format: Option<FileFormat>| {
        let opts = ExportOptions { color_space: space, format_override: format, ..ExportOptions::new(named.clone()) };
        list_outputs(&fx.project, 0, "sphere", &opts).unwrap().remove(0)
    };
    assert!(plan(None, None).name.ends_with("_sRGB.png") && plan(None, Some(FileFormat::Exr)).name.ends_with("_linear.exr"), "defaults keep their names");
    assert!(plan(Some(ColorSpace::LinearRec709), None).name.ends_with("_linear.png"));
    let aces = plan(Some(ColorSpace::AcesCg), None);
    assert!(aces.name.ends_with("_ACEScg.png"), "{}", aces.name);
    assert!(aces.warnings.iter().any(|w| w.contains("clamped")), "{:?}", aces.warnings);
    assert!(plan(Some(ColorSpace::AcesCg), Some(FileFormat::Exr)).warnings.is_empty(), "float files keep the values");
    assert!(plan(None, None).warnings.is_empty() && plan(Some(ColorSpace::LinearRec709), None).warnings.is_empty());
    // The planned names are the rendered names.
    let opts = ExportOptions { color_space: Some(ColorSpace::Aces2065), ..ExportOptions::new(named) };
    let names: Vec<String> = fx.render(&opts).into_iter().map(|f| f.name).collect();
    assert_eq!(names, list_outputs(&fx.project, 0, "sphere", &opts).unwrap().into_iter().map(|p| p.name).collect::<Vec<_>>());
    assert!(names[0].ends_with("_ACES2065-1.png"), "{names:?}");
}

#[test]
fn colour_packed_into_one_channel_warns_instead_of_converting_silently() {
    let fx = Fx::new(&default_channels());
    let preset = ExportPreset { name: "l".into(), maps: vec![gray_map("$textureSet_red", SourceRef::Channel { channel: Channel::BaseColor, component: 0 })] };
    let opts = |space| ExportOptions { color_space: space, format_override: Some(FileFormat::Exr), ..ExportOptions::new(preset.clone()) };
    assert!(fx.render(&opts(Some(ColorSpace::AcesCg)))[0].warnings.iter().any(|w| w.contains("ACEScg")));
    assert!(fx.render(&opts(None))[0].warnings.is_empty(), "linear Rec.709 needs no conversion");
}

/// The ICC profile a PNG, TIFF, JPEG or WebP file carries.
fn embedded_icc(format: FileFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let c = || Cursor::new(bytes);
    match format {
        FileFormat::Png => image::codecs::png::PngDecoder::new(c()).ok()?.icc_profile().ok()?,
        FileFormat::Tiff => image::codecs::tiff::TiffDecoder::new(c()).ok()?.icc_profile().ok()?,
        FileFormat::Jpeg => image::codecs::jpeg::JpegDecoder::new(c()).ok()?.icc_profile().ok()?,
        FileFormat::Webp => image::codecs::webp::WebPDecoder::new(c()).ok()?.icc_profile().ok()?,
        _ => None,
    }
}

#[test]
fn a_chosen_colour_space_is_embedded_as_an_icc_profile() {
    let data = vec![0.25f32; 8 * 8 * 3];
    for format in [FileFormat::Png, FileFormat::Tiff, FileFormat::Jpeg, FileFormat::Webp] {
        for depth in [8u8, 16] {
            if depth == 16 && matches!(format, FileFormat::Jpeg | FileFormat::Webp) {
                continue;
            }
            for space in ColorSpace::ALL {
                let mut warnings = Vec::new();
                let bytes = encode(format, depth, (8, 8), 3, &data, &[false; 3], Some(space), &mut warnings).unwrap();
                assert_eq!(embedded_icc(format, &bytes), crate::icc::profile(space), "{format:?} {depth}-bit {space:?}");
                assert!(warnings.is_empty(), "{warnings:?}");
            }
            // No colour space chosen (a data map, or the default): nothing is embedded.
            let bytes = encode(format, depth, (8, 8), 3, &data, &[false; 3], None, &mut Vec::new()).unwrap();
            assert_eq!(embedded_icc(format, &bytes), None, "{format:?} {depth}-bit");
        }
    }
}

#[test]
fn exr_carries_the_chromaticities_of_the_space() {
    let data = vec![0.25f32; 4 * 4 * 3];
    let read = |bytes: &[u8]| {
        let meta = exr::meta::MetaData::read_from_buffered(Cursor::new(bytes), false).unwrap();
        meta.headers[0].shared_attributes.chromaticities
    };
    assert!(read(&encode(FileFormat::Exr, 16, (4, 4), 3, &data, &[false; 3], None, &mut Vec::new()).unwrap()).is_none());
    for space in ColorSpace::ALL {
        let c = read(&encode(FileFormat::Exr, 32, (4, 4), 3, &data, &[false; 3], Some(space), &mut Vec::new()).unwrap()).unwrap_or_else(|| panic!("{space:?}"));
        let [r, g, b, w] = space.chromaticities();
        for (got, want) in [(c.red, r), (c.green, g), (c.blue, b), (c.white, w)] {
            assert!((f64::from(got.0) - want[0]).abs() < 1e-5 && (f64::from(got.1) - want[1]).abs() < 1e-5, "{space:?}: {got:?} vs {want:?}");
        }
    }
}

#[test]
fn formats_without_a_place_for_a_colour_space_say_so() {
    let data = vec![0.25f32; 4 * 4 * 3];
    for format in [FileFormat::Tga, FileFormat::Bmp, FileFormat::Hdr] {
        let mut warnings = Vec::new();
        encode(format, 8, (4, 4), 3, &data, &[false; 3], Some(ColorSpace::AcesCg), &mut warnings).unwrap();
        assert!(warnings.iter().any(|w| w.contains("colour space") && w.contains("ACEScg")), "{format:?}: {warnings:?}");
        let mut quiet = Vec::new();
        encode(format, 8, (4, 4), 3, &data, &[false; 3], None, &mut quiet).unwrap();
        assert!(quiet.is_empty(), "{format:?}: nothing to say without a chosen space: {quiet:?}");
    }
}
