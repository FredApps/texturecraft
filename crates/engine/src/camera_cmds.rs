//! The camera list (the viewport camera menu: the default camera plus saved and imported
//! cameras, saved with the project) and the depth of field focus pick.

use serde_json::{Value, json};
use texturecraft_core::Camera;
use texturecraft_geom::OrbitCamera;
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::commands::{CommandSpec, always, bad, f32p, strp, u64p};
use crate::tool_cmds::viewport_of;
use crate::{Result, Session, cmd};

/// Name of the viewport's own camera in the camera menu (it is not in the list).
pub const DEFAULT_CAMERA: &str = "Default camera";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("camera.list", "Cameras", [], None, "{}", always, |s, _| Ok(list(s))),
        cmd!("camera.add", "Save view as camera", [], None, "{name?}", always, add),
        cmd!(
            "camera.select",
            "Select camera",
            [],
            None,
            "{index? | name?} (neither, or name \"Default camera\": the default camera framing the mesh)",
            always,
            select
        ),
        cmd!("camera.remove", "Remove camera", [], None, "{index? | name?}", has_cameras, remove),
        cmd!("camera.rename", "Rename camera", [], None, "{index? | name?, to}", has_cameras, rename),
        cmd!("camera.bookmark1", "Go to camera 1", ["Viewport"], Some("Ctrl+1"), "{}", always, |s, _| bookmark(s, 0)),
        cmd!("camera.bookmark2", "Go to camera 2", ["Viewport"], Some("Ctrl+2"), "{}", always, |s, _| bookmark(s, 1)),
        cmd!("camera.bookmark3", "Go to camera 3", ["Viewport"], Some("Ctrl+3"), "{}", always, |s, _| bookmark(s, 2)),
        cmd!("camera.bookmark4", "Go to camera 4", ["Viewport"], Some("Ctrl+4"), "{}", always, |s, _| bookmark(s, 3)),
        cmd!("camera.bookmark5", "Go to camera 5", ["Viewport"], Some("Ctrl+5"), "{}", always, |s, _| bookmark(s, 4)),
        cmd!("camera.bookmark6", "Go to camera 6", ["Viewport"], Some("Ctrl+6"), "{}", always, |s, _| bookmark(s, 5)),
        cmd!("camera.bookmark7", "Go to camera 7", ["Viewport"], Some("Ctrl+7"), "{}", always, |s, _| bookmark(s, 6)),
        cmd!("camera.bookmark8", "Go to camera 8", ["Viewport"], Some("Ctrl+8"), "{}", always, |s, _| bookmark(s, 7)),
        cmd!("camera.bookmark9", "Go to camera 9", ["Viewport"], Some("Ctrl+9"), "{}", always, |s, _| bookmark(s, 8)),
        cmd!("viewport.focusAt", "Focus depth of field here", [], None, "{x, y (viewport pixels), viewport?: [w, h]}", always, focus_at),
    ]
}

fn has_cameras(s: &Session) -> std::result::Result<(), String> {
    if s.doc.project.cameras.is_empty() { Err("the project has no saved cameras".into()) } else { Ok(()) }
}

/// The view as a saved camera.
fn saved(c: &OrbitCamera, name: String) -> Camera {
    Camera { name, target: c.target.to_array(), yaw: c.yaw, pitch: c.pitch, distance: c.distance, fov: c.fov, ortho: c.ortho }
}

/// Move the view to saved camera `c` (values from project files are checked and clamped).
fn apply(view: &mut OrbitCamera, c: &Camera) {
    let target = Vec3::from(c.target);
    if target.is_finite() {
        view.target = target;
    }
    let r = view.scene_radius.max(1e-3);
    let ok = |v: f32| v.is_finite().then_some(v);
    view.yaw = ok(c.yaw).unwrap_or(view.yaw);
    view.pitch = ok(c.pitch).map_or(view.pitch, |p| p.clamp(-89f32.to_radians(), 89f32.to_radians()));
    view.distance = ok(c.distance).map_or(view.distance, |d| d.clamp(r * 0.05, r * 200.0));
    view.fov = ok(c.fov).map_or(view.fov, |f| f.clamp(5.0, 150.0));
    view.ortho = c.ortho;
}

/// The saved camera the view is at, if any.
pub fn current(s: &Session) -> Option<usize> {
    let mut view = s.camera;
    s.doc.project.cameras.iter().position(|c| {
        apply(&mut view, c);
        let close = |a: f32, b: f32| (a - b).abs() <= 1e-4 * (1.0 + a.abs());
        (view.target - s.camera.target).length() <= 1e-4 * (1.0 + view.distance)
            && close(view.yaw, s.camera.yaw)
            && close(view.pitch, s.camera.pitch)
            && close(view.distance, s.camera.distance)
            && close(view.fov, s.camera.fov)
            && view.ortho == s.camera.ortho
    })
}

fn list(s: &Session) -> Value {
    let cams: Vec<Value> = s
        .doc
        .project
        .cameras
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "index": i, "name": c.name, "fov": c.fov, "ortho": c.ortho, "distance": c.distance,
                "yaw": c.yaw.to_degrees(), "pitch": c.pitch.to_degrees(), "target": c.target,
            })
        })
        .collect();
    json!({ "cameras": cams, "current": current(s) })
}

/// The camera `index` or `name` of params `p` points at.
fn which(s: &Session, cmd: &str, p: &Value) -> Result<usize> {
    let cams = &s.doc.project.cameras;
    let found = match (u64p(p, "index"), strp(p, "name")) {
        (Some(i), _) => usize::try_from(i).ok().filter(|i| *i < cams.len()),
        (None, Some(n)) => cams.iter().position(|c| c.name == n),
        (None, None) => return Err(bad(cmd, "give `index` or `name`")),
    };
    found.ok_or_else(|| bad(cmd, "no such camera (see camera.list)"))
}

fn unique_name(s: &Session, base: &str) -> String {
    let taken = |n: &str| n == DEFAULT_CAMERA || s.doc.project.cameras.iter().any(|c| c.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_default()
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let base = strp(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Camera");
    let name = unique_name(s, base);
    let cam = saved(&s.camera, name.clone());
    s.edit("Save camera", |s| {
        s.doc.project.cameras.push(cam);
        Ok(json!({ "index": s.doc.project.cameras.len() - 1, "name": name }))
    })
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    if u64p(p, "index").is_none() && strp(p, "name").is_none_or(|n| n == DEFAULT_CAMERA) {
        // The default camera: fov and projection from the display settings, framing the mesh.
        s.camera.fov = s.doc.project.display.fov;
        s.camera.ortho = false;
        return crate::execute(s, "viewport.frame", &Value::Null);
    }
    let i = which(s, "camera.select", p)?;
    if let Some(c) = s.doc.project.cameras.get(i).cloned() {
        apply(&mut s.camera, &c);
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(crate::view_cmds::camera_json(s))
}

/// Shortcut slot `slot` (0-based): the n-th saved camera, in list order.
fn bookmark(s: &mut Session, slot: usize) -> Result<Value> {
    if slot >= s.doc.project.cameras.len() {
        return Err(bad("camera.bookmark", "no saved camera in that slot (see camera.list)"));
    }
    select(s, &json!({ "index": slot }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = which(s, "camera.remove", p)?;
    s.edit("Remove camera", |s| {
        let c = s.doc.project.cameras.remove(i);
        Ok(json!({ "removed": c.name }))
    })
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let i = which(s, "camera.rename", p)?;
    let to = strp(p, "to").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("camera.rename", "`to` (the new name) is required"))?;
    if s.doc.project.cameras.get(i).is_some_and(|c| c.name == to) {
        return Ok(json!({ "index": i, "name": to }));
    }
    let name = unique_name(s, to);
    s.edit("Rename camera", |s| {
        if let Some(c) = s.doc.project.cameras.get_mut(i) {
            c.name = name.clone();
        }
        Ok(json!({ "index": i, "name": name }))
    })
}

/// Focus the depth of field on the mesh under viewport pixel (x, y).
fn focus_at(s: &mut Session, p: &Value) -> Result<Value> {
    let (Some(x), Some(y)) = (f32p(p, "x"), f32p(p, "y")) else { return Err(bad("viewport.focusAt", "`x` and `y` (viewport pixels) are required")) };
    let vp = viewport_of(s, p);
    let hit = s.pick(Vec2::new(x, y), vp).ok_or_else(|| bad("viewport.focusAt", "no mesh under that point"))?;
    let depth = (hit.pos - s.camera.eye()).dot(s.camera.forward()).max(1e-4);
    s.doc.project.display.dof.focus_distance = depth;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "focusDistance": depth }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute;

    fn session() -> Session {
        Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 24, 12), "builtin:sphere", None, 128).unwrap()
    }

    #[test]
    fn cameras_add_select_rename_remove_and_undo() {
        let mut s = session();
        let home = s.camera;
        assert_eq!(execute(&mut s, "camera.list", &Value::Null).unwrap()["current"], Value::Null);
        execute(&mut s, "camera.set", &json!({"yaw": 30, "pitch": 10, "fov": 25, "ortho": true})).unwrap();
        let side = s.camera;
        assert_eq!(execute(&mut s, "camera.add", &json!({"name": "Side"})).unwrap()["index"], 0);
        assert_eq!(execute(&mut s, "camera.add", &json!({"name": "Side"})).unwrap()["name"], "Side 2", "names stay unique");
        assert_eq!(execute(&mut s, "camera.list", &Value::Null).unwrap()["current"], 0);
        execute(&mut s, "camera.select", &json!({})).unwrap();
        assert!(!s.camera.ortho && s.camera.fov == s.doc.project.display.fov, "the default camera");
        assert_eq!((s.camera.yaw, s.camera.target), (side.yaw, home.target), "framing the mesh, keeping the angles");
        execute(&mut s, "camera.orbit", &json!({"dx": 40})).unwrap();
        execute(&mut s, "camera.select", &json!({"name": "Side"})).unwrap();
        assert_eq!(s.camera, side, "selecting brings back the saved view, fov and projection");
        execute(&mut s, "camera.rename", &json!({"index": 1, "to": "Top"})).unwrap();
        execute(&mut s, "camera.remove", &json!({"name": "Side"})).unwrap();
        let list = execute(&mut s, "camera.list", &Value::Null).unwrap();
        assert_eq!(list["cameras"].as_array().unwrap().len(), 1);
        assert_eq!(list["cameras"][0]["name"], "Top");
        assert!(execute(&mut s, "camera.select", &json!({"index": 5})).is_err());
        assert!(execute(&mut s, "camera.rename", &json!({"index": 0})).is_err(), "`to` is required");
        execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.project.cameras.len(), 2, "removing is undoable");
    }

    #[test]
    fn number_shortcuts_go_to_the_first_nine_cameras() {
        let mut s = session();
        assert!(execute(&mut s, "camera.bookmark1", &Value::Null).is_err(), "no cameras yet");
        execute(&mut s, "camera.set", &json!({"yaw": 30})).unwrap();
        let first = s.camera;
        execute(&mut s, "camera.add", &json!({"name": "A"})).unwrap();
        execute(&mut s, "camera.orbit", &json!({"dx": 50})).unwrap();
        execute(&mut s, "camera.add", &json!({"name": "B"})).unwrap();
        execute(&mut s, "camera.bookmark1", &Value::Null).unwrap();
        assert_eq!(s.camera, first);
        execute(&mut s, "camera.bookmark2", &Value::Null).unwrap();
        assert_eq!(execute(&mut s, "camera.list", &Value::Null).unwrap()["current"], 1);
        assert!(execute(&mut s, "camera.bookmark3", &Value::Null).is_err(), "empty slot");
        let ids: Vec<_> = crate::command_specs().iter().filter(|c| c.id.starts_with("camera.bookmark")).collect();
        assert_eq!(ids.len(), 9);
        assert!(ids.iter().all(|c| c.shortcut.is_some()));
    }

    #[test]
    fn cameras_round_trip_through_the_project_file() {
        let mut s = session();
        execute(&mut s, "camera.set", &json!({"yaw": -20, "fov": 60})).unwrap();
        execute(&mut s, "camera.add", &json!({"name": "Hero"})).unwrap();
        execute(&mut s, "display.set", &json!({"dof": {"enabled": true, "focusDistance": 3.5, "aperture": 0.2}, "ior": 1.33})).unwrap();
        let bytes = texturecraft_formats::save_project_bytes(&s.project_file()).unwrap();
        let back = Session::from_project_bytes(&bytes).unwrap();
        assert_eq!(back.doc.project.cameras, s.doc.project.cameras);
        assert_eq!(back.doc.project.display.dof, s.doc.project.display.dof);
        assert_eq!(back.doc.project.display.ior, 1.33);
        // Projects saved before cameras and depth of field existed load with the defaults.
        let mut v = serde_json::to_value(&s.doc.project).unwrap();
        v.as_object_mut().unwrap().remove("cameras");
        v["display"].as_object_mut().unwrap().remove("dof");
        let old: texturecraft_core::Project = serde_json::from_value(v).unwrap();
        assert!(old.cameras.is_empty() && !old.display.dof.enabled);
    }

    #[test]
    fn focus_at_picks_the_depth_under_the_cursor() {
        let mut s = session();
        let vp = s.viewport;
        let r = execute(&mut s, "viewport.focusAt", &json!({"x": vp.x / 2.0, "y": vp.y / 2.0})).unwrap();
        let d = r["focusDistance"].as_f64().unwrap() as f32;
        // The front of the unit sphere, one radius before the orbit target.
        assert!((d - (s.camera.distance - 1.0)).abs() < 0.05, "{d} vs {}", s.camera.distance);
        assert_eq!(s.doc.project.display.dof.focus_distance, d);
        assert!(execute(&mut s, "viewport.focusAt", &json!({"x": 1, "y": 1})).is_err(), "nothing under the corner");
    }
}
