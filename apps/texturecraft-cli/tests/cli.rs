//! Drives the `texturecraft-cli` binary.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

// Nextest 0.9.130+ remaps this variable when an archive is extracted on another
// runner. Cargo's compile-time path still supports ordinary `cargo test`.
fn cli_binary() -> OsString {
    cli_binary_path(std::env::var_os("CARGO_BIN_EXE_texturecraft-cli"))
}

fn cli_binary_path(runner_path: Option<OsString>) -> OsString {
    runner_path.unwrap_or_else(|| env!("CARGO_BIN_EXE_texturecraft-cli").into())
}

#[test]
fn cli_binary_prefers_relocated_runner_path() {
    let relocated = OsString::from("relocated archive/texturecraft-cli");
    assert_eq!(cli_binary_path(Some(relocated.clone())), relocated);
}

#[test]
fn cli_binary_defaults_to_cargo_path() {
    assert_eq!(cli_binary_path(None), OsString::from(env!("CARGO_BIN_EXE_texturecraft-cli")));
}

fn cli(args: &[&str]) -> Output {
    Command::new(cli_binary()).args(args).output().unwrap()
}

fn ok(args: &[&str]) -> String {
    let o = cli(args);
    assert!(o.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

fn json(args: &[&str]) -> Value {
    serde_json::from_str(&ok(args)).unwrap()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tc-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn version_and_usage_errors() {
    let v = ok(&["--version"]);
    assert!(v.starts_with("TextureCraft "), "{v}");
    assert_eq!(v.lines().count(), 1);
    assert!(ok(&["--help"]).contains("texturecraft-cli mcp"));
    assert_eq!(cli(&[]).status.code(), Some(2));
    assert_eq!(cli(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(cli(&["info", "--bogus"]).status.code(), Some(2));
    let o = cli(&["info", "does/not/exist.tcraft"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error"));
    assert_eq!(cli(&["run", "builtin:nope", "layer.add.fill"]).status.code(), Some(1));
    assert_eq!(cli(&["run", "builtin:cube", "layer.add.fill", "{not json"]).status.code(), Some(1));
    assert_eq!(cli(&["render", "builtin:cube", "x.png", "--size", "big"]).status.code(), Some(1));
}

#[test]
fn new_info_run_bake_export_batch() {
    let d = tmp("flow");
    let a = d.join("a.tcraft");
    ok(&["new", "builtin:cube", p(&a), "--resolution", "128"]);
    let info = json(&["info", p(&a)]);
    assert_eq!(info["kind"], "project");
    assert_eq!(info["textureSets"][0]["resolution"], 128);
    assert!(info["mesh"]["triangles"].as_u64().unwrap() >= 12);

    let b = d.join("b.tcraft");
    let r = json(&["run", p(&a), "layer.add.fill", r#"{"color":[1,0,0],"roughness":0.3}"#, "-o", p(&b)]);
    assert!(r["layer"].is_u64(), "{r}");
    assert_eq!(json(&["info", p(&b)])["layers"][0]["kind"], "fill");

    ok(&["bake", p(&b), "--resolution", "128", "--maps", "ambientOcclusion,curvature"]);
    let maps = json(&["info", p(&b)])["textureSets"][0]["baked"]["maps"].clone();
    assert_eq!(maps.as_array().unwrap().len(), 2, "{maps}");

    let tex = d.join("tex");
    let files = ok(&["export", p(&b), p(&tex), "--preset", "glTF PBR Metal Roughness", "--size", "128"]);
    assert!(files.lines().count() >= 3, "{files}");
    for f in files.lines() {
        assert!(Path::new(f).is_file(), "{f}");
    }

    let script = d.join("script.json");
    let c = d.join("c.tcraft");
    let body = serde_json::json!({"open": p(&b), "steps": [{"id": "layer.add.paint", "params": {"name": "Scripted"}}], "save": p(&c)});
    std::fs::write(&script, body.to_string()).unwrap();
    assert_eq!(json(&["batch", p(&script)])["completed"], 1);
    assert_eq!(json(&["info", p(&c)])["layers"][0]["name"], "Scripted");
    std::fs::write(&script, r#"[{"id":"layer.add.paint"},{"id":"no.such"}]"#).unwrap();
    assert_eq!(cli(&["batch", p(&script)]).status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn info_on_a_mesh_reports_uvs() {
    let d = tmp("mesh");
    let obj = d.join("quad.obj");
    std::fs::write(&obj, "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl Paint\nf 1/1 2/2 3/3 4/4\n").unwrap();
    let info = json(&["info", p(&obj)]);
    assert_eq!(info["kind"], "mesh");
    assert_eq!(info["triangles"], 2);
    assert_eq!(info["materials"][0], "Paint");
    assert_eq!(info["uv"]["uvsGenerated"], false);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn serve_takes_the_automation_root_spelling_too() {
    use std::io::Write as _;
    let d = tmp("automation-root");
    let mut child = Command::new(cli_binary())
        .args(["serve", "--automation-write-root", p(&d)])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{\"id\":1,\"method\":\"project.save\",\"params\":{\"path\":\"a.tcraft\"}}\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(d.join("a.tcraft").is_file());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn info_on_a_builtin_mesh() {
    let info = json(&["info", "builtin:cube"]);
    assert_eq!(info["kind"], "mesh");
    assert_eq!(info["file"], "builtin:cube");
    assert_eq!(info["triangles"], 1728);
    assert_eq!(info["uv"]["uvsGenerated"], false);
    assert_eq!(cli(&["info", "builtin:bogus"]).status.code(), Some(1));
}

#[test]
fn render_writes_a_png() {
    let d = tmp("render");
    let out = d.join("ball.png");
    let o = cli(&["render", "builtin:material_ball", p(&out), "--size", "160x120", "--yaw", "30"]);
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains("cannot render"), "{err}");
        eprintln!("skipping: {err}");
        return;
    }
    let img = image::open(&out).unwrap();
    assert_eq!((img.width(), img.height()), (160, 120));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn path_traced_render_needs_no_gpu() {
    let d = tmp("trace");
    let out = d.join("ball.png");
    // On the GPU where it has compute shaders, then forced onto the CPU.
    for extra in [None, Some("--cpu")] {
        let mut args = vec!["render", "builtin:material_ball", p(&out), "--size", "96x64", "--samples", "4"];
        args.extend(extra);
        ok(&args);
        let img = image::open(&out).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (96, 64));
        // The lit ball in the middle is brighter than the dark backdrop corner.
        let lum = |x, y| img.get_pixel(x, y).0.iter().map(|v| *v as u32).sum::<u32>();
        assert!(lum(48, 28) > lum(1, 1) + 60);
    }
    // --camera looks through a camera saved in the project; an unknown name is an error.
    let proj = d.join("cams.tcraft");
    ok(&["run", "builtin:cube", "camera.set", r#"{"yaw": 80, "distance": 9}"#, "-o", p(&proj)]);
    ok(&["run", p(&proj), "camera.add", r#"{"name": "Side"}"#, "-o", p(&proj)]);
    ok(&["render", p(&proj), p(&out), "--size", "48x32", "--camera", "Side", "--samples", "2", "--cpu"]);
    assert_eq!(cli(&["render", p(&proj), p(&out), "--camera", "Nope", "--samples", "2", "--cpu"]).status.code(), Some(1));
    assert_eq!(cli(&["render", "builtin:cube", p(&out), "--samples", "0"]).status.code(), Some(1));
    assert_eq!(cli(&["render", "builtin:cube", p(&out), "--samples", "4", "--view", "2d"]).status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&d);
}
