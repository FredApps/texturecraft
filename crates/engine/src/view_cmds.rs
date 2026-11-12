//! Edit, viewport and display commands.

use serde_json::{Value, json};
use texturecraft_core::project::ENVIRONMENTS;
use texturecraft_core::{BakedMap, Channel, ToneMapping};
use texturecraft_geom::glam::Vec3;

use crate::commands::{CommandSpec, always, bad, boolp, can_redo, can_undo, color_param, f32p, strp, u64p};
use crate::session::ViewMode;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("viewport.frame", "Show entire mesh", ["Viewport"], Some("F"), "{}", always, frame),
        cmd!("viewport.displayMaterial", "Display material", ["Viewport"], Some("M"), "{}", always, |s, _| set_view(s, ViewMode::Material)),
        cmd!("viewport.nextChannel", "Display next channel", ["Viewport"], Some("C"), "{}", always, |s, _| cycle_channel(s, 1)),
        cmd!("viewport.previousChannel", "Display previous channel", ["Viewport"], Some("Shift+C"), "{}", always, |s, _| cycle_channel(s, -1)),
        cmd!("viewport.nextMeshMap", "Display next mesh map", ["Viewport"], Some("B"), "{}", always, |s, _| cycle_map(s, 1)),
        cmd!("viewport.previousMeshMap", "Display previous mesh map", ["Viewport"], Some("Shift+B"), "{}", always, |s, _| cycle_map(s, -1)),
        cmd!("viewport.view", "Set view mode", [], None, "{mode: material|uvChecker|mask|<channel id>|map:<mesh map id>, layer? (for mask)}", always, view),
        cmd!("camera.set", "Set camera", [], None, "{yaw?, pitch? (degrees), distance?, fov?, target?: [x,y,z], ortho?}", always, camera_set),
        cmd!("camera.get", "Get camera", [], None, "{}", always, |s, _| Ok(camera_json(s))),
        cmd!("camera.orbit", "Orbit camera", [], None, "{dx?, dy? (degrees)}", always, orbit),
        cmd!(
            "display.set",
            "Display settings",
            [],
            None,
            "{environment?: <built-in>|resource:<image resource>, exposure?, blur?, rotation?, opacity?, toneMapping?: linear|aces, fov?, shadows?, aoIntensity?, bloom?, vignette?, sharpen?, grain?, postEffects?, taa?, specGloss?, parallax?, displacement? (the viewport moves the surface by the height channel), scatterColor?: [r,g,b] (linear) | \"#rrggbb\", scatterRadius? (0..1 of the mesh radius), ior? (1..3, transmission), dof?: {enabled?, focusDistance? (view depth, world units; 0 = the orbit target), aperture? (lens radius, 0..1 of the mesh radius)}}",
            always,
            display
        ),
    ]
}

pub(crate) fn edit_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.undo",
            "Undo",
            ["Edit"],
            Some("Ctrl+Z"),
            "{steps?: 1 (as many as there are)} → {undone: the last step undone, steps}",
            can_undo,
            |s, p| {
                let (last, n) = repeat(s, p, Session::undo)?;
                Ok(json!({ "undone": last, "steps": n }))
            }
        ),
        cmd!(
            "edit.redo",
            "Redo",
            ["Edit"],
            Some("Ctrl+Y"),
            "{steps?: 1 (as many as there are)} → {redone: the last step redone, steps}",
            can_redo,
            |s, p| {
                let (last, n) = repeat(s, p, Session::redo)?;
                Ok(json!({ "redone": last, "steps": n }))
            }
        ),
        cmd!("edit.history", "History", [], None, "{} → {undo: steps, oldest first, redo: undone steps, next first}", always, |s, _| Ok(
            json!({ "undo": s.history_labels(), "redo": s.redo_labels() })
        )),
    ]
}

/// Run `step` (undo or redo) `steps` times (default 1) or until the history runs out: the last
/// step's label and how many ran (the History panel jumps to a step this way).
fn repeat(s: &mut Session, p: &Value, step: fn(&mut Session) -> Result<String>) -> Result<(String, u64)> {
    let steps = u64p(p, "steps").unwrap_or(1).max(1);
    let mut last = step(s)?;
    let mut n = 1;
    while n < steps
        && let Ok(l) = step(s)
    {
        last = l;
        n += 1;
    }
    Ok((last, n))
}

fn frame(s: &mut Session, _: &Value) -> Result<Value> {
    let (lo, hi) = s.mesh.bounds();
    s.camera.frame(lo, hi);
    s.revision = s.revision.wrapping_add(1);
    Ok(camera_json(s))
}

fn set_view(s: &mut Session, v: ViewMode) -> Result<Value> {
    if s.bake_preview.is_some() {
        // During a bake the view follows the preview: a mesh map pins it, another mode comes back afterwards
        s.bake_preview_user_view(v);
    } else {
        s.view = v;
    }
    let v = s.view;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "view": v.label() }))
}

fn cycle_channel(s: &mut Session, d: i64) -> Result<Value> {
    let chans = s.doc.project.texture_set(s.ts()).map(|t| t.channel_list()).unwrap_or_default();
    if chans.is_empty() {
        return set_view(s, ViewMode::Material);
    }
    let n = chans.len() as i64;
    let i = match s.view {
        ViewMode::Channel(c) => chans.iter().position(|x| *x == c).map(|i| (i as i64 + d).rem_euclid(n)).unwrap_or(0),
        _ => {
            if d > 0 {
                0
            } else {
                n - 1
            }
        }
    };
    set_view(s, ViewMode::Channel(chans[i as usize]))
}

fn cycle_map(s: &mut Session, d: i64) -> Result<Value> {
    let maps = BakedMap::ALL;
    let n = maps.len() as i64;
    let i = match s.view {
        ViewMode::MeshMap(m) => maps.iter().position(|x| *x == m).map(|i| (i as i64 + d).rem_euclid(n)).unwrap_or(0),
        _ => {
            if d > 0 {
                0
            } else {
                n - 1
            }
        }
    };
    set_view(s, ViewMode::MeshMap(maps[i as usize]))
}

fn view(s: &mut Session, p: &Value) -> Result<Value> {
    let m = strp(p, "mode").ok_or_else(|| bad("viewport.view", "`mode` is required"))?;
    let v = match m {
        "material" => ViewMode::Material,
        "uvChecker" | "uv" | "checker" => ViewMode::UvChecker,
        "mask" => {
            let id = crate::commands::layer_param(s, "viewport.view", p)?;
            if s.doc.project.layer(id)?.mask.is_none() {
                return Err(bad("viewport.view", "the layer has no mask"));
            }
            ViewMode::Mask(id)
        }
        _ => match m.strip_prefix("map:") {
            Some(id) => ViewMode::MeshMap(BakedMap::parse(id).ok_or_else(|| bad("viewport.view", format!("unknown mesh map `{id}`")))?),
            None => ViewMode::Channel(Channel::parse(m).ok_or_else(|| bad("viewport.view", format!("unknown view mode `{m}`")))?),
        },
    };
    set_view(s, v)
}

pub(crate) fn camera_json(s: &Session) -> Value {
    let c = &s.camera;
    json!({
        "yaw": c.yaw.to_degrees(), "pitch": c.pitch.to_degrees(), "distance": c.distance, "fov": c.fov,
        "target": c.target.to_array(), "eye": c.eye().to_array(), "ortho": c.ortho,
    })
}

fn camera_set(s: &mut Session, p: &Value) -> Result<Value> {
    let c = &mut s.camera;
    if let Some(v) = f32p(p, "yaw") {
        c.yaw = v.to_radians();
    }
    if let Some(v) = f32p(p, "pitch") {
        c.pitch = v.clamp(-89.0, 89.0).to_radians();
    }
    if let Some(v) = f32p(p, "distance") {
        c.distance = v.clamp(c.scene_radius * 0.05, c.scene_radius * 200.0);
    }
    if let Some(v) = f32p(p, "fov") {
        c.fov = v.clamp(5.0, 150.0);
    }
    if let Some(t) = p.get("target").and_then(Value::as_array) {
        let v: Vec<f32> = t.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
        if v.len() == 3 && v.iter().all(|x| x.is_finite()) {
            c.target = Vec3::new(v[0], v[1], v[2]);
        }
    }
    if let Some(o) = boolp(p, "ortho") {
        c.ortho = o;
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(camera_json(s))
}

fn orbit(s: &mut Session, p: &Value) -> Result<Value> {
    s.camera.orbit(f32p(p, "dx").unwrap_or(0.0).to_radians(), f32p(p, "dy").unwrap_or(0.0).to_radians());
    s.revision = s.revision.wrapping_add(1);
    Ok(camera_json(s))
}

fn display(s: &mut Session, p: &Value) -> Result<Value> {
    let tm = match strp(p, "toneMapping") {
        Some("linear") => Some(ToneMapping::Linear),
        Some("aces") | Some("acesFilmic") => Some(ToneMapping::AcesFilmic),
        Some(o) => return Err(bad("display.set", format!("unknown tone mapping `{o}`"))),
        None => None,
    };
    let env = strp(p, "environment").map(str::to_string);
    if let Some(name) = env.as_deref().and_then(|e| e.strip_prefix(texturecraft_core::project::ENV_RESOURCE_PREFIX))
        && !s.images.contains_key(name)
    {
        return Err(bad("display.set", format!("no image resource `{name}` (import it with resource.import)")));
    }
    if let Some(e) = env.as_deref().filter(|e| !e.starts_with(texturecraft_core::project::ENV_RESOURCE_PREFIX) && !ENVIRONMENTS.contains(e)) {
        return Err(bad("display.set", format!("unknown environment `{e}`: one of {}, or resource:<image>", ENVIRONMENTS.join(", "))));
    }
    let scatter_color = color_param("display.set", p, "scatterColor")?;
    let d = &mut s.doc.project.display;
    if let Some(e) = env {
        d.environment = e;
    }
    for (k, slot, lo, hi) in [
        ("exposure", &mut d.env_exposure, -10.0, 10.0),
        ("blur", &mut d.env_blur, 0.0, 1.0),
        ("rotation", &mut d.env_rotation, -360.0, 360.0),
        ("opacity", &mut d.env_opacity, 0.0, 1.0),
        ("fov", &mut d.fov, 5.0, 150.0),
        ("aoIntensity", &mut d.ao_intensity, 0.0, 1.0),
        ("bloom", &mut d.bloom, 0.0, 1.0),
        ("vignette", &mut d.vignette, 0.0, 1.0),
        ("sharpen", &mut d.sharpen, 0.0, 1.0),
        ("grain", &mut d.grain, 0.0, 1.0),
        ("scatterRadius", &mut d.scatter_radius, 0.0, 1.0),
        ("ior", &mut d.ior, 1.0, 3.0),
    ] {
        if let Some(v) = f32p(p, k) {
            *slot = v.clamp(lo, hi);
        }
    }
    if let Some(t) = tm {
        d.tone_mapping = t;
    }
    if let Some(c) = scatter_color {
        d.scatter_color = c.map(|v| v.clamp(0.0, 1.0));
    }
    if let Some(b) = boolp(p, "shadows") {
        d.shadows = b;
    }
    if let Some(dof) = p.get("dof") {
        if let Some(b) = boolp(dof, "enabled") {
            d.dof.enabled = b;
        }
        if let Some(v) = f32p(dof, "focusDistance") {
            d.dof.focus_distance = v.clamp(0.0, 1e6);
        }
        if let Some(v) = f32p(dof, "aperture") {
            d.dof.aperture = v.clamp(0.0, 1.0);
        }
    }
    for (k, slot) in [
        ("postEffects", &mut d.post_effects),
        ("taa", &mut d.taa),
        ("specGloss", &mut d.spec_gloss),
        ("parallax", &mut d.parallax),
        ("displacement", &mut d.displacement),
    ] {
        if let Some(b) = boolp(p, k) {
            *slot = b;
        }
    }
    // The field of view applies to the view only when set (a saved camera keeps its own).
    if f32p(p, "fov").is_some() {
        s.camera.fov = s.doc.project.display.fov;
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(serde_json::to_value(&s.doc.project.display).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute;

    #[test]
    fn hdr_resources_become_the_environment() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
        assert!(execute(&mut s, "display.set", &json!({"environment": "resource:sky.hdr"})).is_err(), "unknown resources are rejected");
        let e = execute(&mut s, "display.set", &json!({"environment": "nonexistent"})).unwrap_err().to_string();
        assert!(e.contains("Studio Soft"), "unknown environments are rejected with the valid names: {e}");
        assert_eq!(s.doc.project.display.environment, "Studio Soft");
        execute(&mut s, "display.set", &json!({"environment": "Night"})).unwrap();
        let img = image::Rgb32FImage::from_fn(64, 32, |_, y| image::Rgb([if y < 8 { 40.0 } else { 0.5 }; 3]));
        let mut hdr = Vec::new();
        image::DynamicImage::ImageRgb32F(img).write_to(&mut std::io::Cursor::new(&mut hdr), image::ImageFormat::Hdr).unwrap();
        let name = s.import_resource_bytes("sky.hdr", hdr).unwrap();
        let res = &s.images[&name];
        assert!(!res.srgb && res.rgba[0][0] > 30.0, "HDR keeps values above 1");
        let env = format!("resource:{name}");
        let r = execute(
            &mut s,
            "display.set",
            &json!({"environment": env, "specGloss": true, "parallax": true, "displacement": true, "taa": false, "scatterColor": [2.0, 0.5, 0.25], "scatterRadius": 0.3}),
        )
        .unwrap();
        assert_eq!(r["environment"], json!(env));
        let d = &s.doc.project.display;
        assert!(d.spec_gloss && d.parallax && d.displacement && !d.taa);
        assert_eq!((d.scatter_color, d.scatter_radius), ([1.0, 0.5, 0.25], 0.3), "scatter settings (clamped)");
        assert_eq!(d.environment_resource(), Some(name.as_str()));
        execute(&mut s, "file.removeUnused", &Value::Null).unwrap();
        assert!(s.doc.resources.contains_key(&name), "the environment's image counts as used");
    }
}
