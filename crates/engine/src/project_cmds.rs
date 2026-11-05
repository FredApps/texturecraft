//! Project-level commands: new project, sample project, smart materials, inspection.

use serde_json::{Value, json};
use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakeResolution, CurvatureMethod};
use texturecraft_geom::primitives;

use crate::commands::{CommandSpec, always, bad, boolp, strp, u64p};
use crate::materials::Place;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "project.new",
            "New project from a built-in mesh",
            [],
            None,
            "{mesh?: material_ball|sphere|cube|rounded_cube|cylinder|torus|plane|crate|udim_plane, resolution?: 1024 (or the template's), name?, uvTiles?: bool (UDIM; default: when the UVs leave tile 1001), template?: project template name (see project.templates)}",
            always,
            new
        ),
        cmd!("project.sample", "Open sample project", ["File"], None, "{}", always, sample),
        cmd!("project.inspect", "Inspect project", [], None, "{}", always, |s, _| Ok(crate::inspect::session(s))),
        cmd!(
            "material.apply",
            "Add smart material",
            [],
            None,
            "{material: id or label, above?: layer id, inside?: bool, index?: n} — above the selection by default; `above` (+ `inside` for a folder) or `index` in the top-level stack (0 = bottom) place it",
            always,
            material
        ),
        cmd!("material.list", "List materials", [], None, "{}", always, |_, _| Ok(json!(
            crate::materials::MATERIALS.iter().map(|m| json!({"id": m.id, "label": m.label, "smart": m.smart})).collect::<Vec<_>>()
        ))),
    ]
}

fn new(s: &mut Session, p: &Value) -> Result<Value> {
    let mesh_id = strp(p, "mesh").unwrap_or("material_ball");
    let mesh = primitives::builtin(mesh_id).ok_or_else(|| bad("project.new", format!("unknown built-in mesh `{mesh_id}`")))?;
    let template = crate::template_cmds::template_param(s, p, "project.new")?;
    let res = u64p(p, "resolution").or(template.as_ref().map(|t| u64::from(t.resolution))).unwrap_or(1024);
    let name = strp(p, "name").unwrap_or("Untitled");
    let mut next = Session::new_project_tiled(name, mesh, &format!("builtin:{mesh_id}"), None, u32::try_from(res).unwrap_or(0), boolp(p, "uvTiles"))?;
    if let Some(t) = &template {
        next.apply_template(t);
    }
    s.switch_to(next);
    Ok(crate::inspect::session(s))
}

fn material(s: &mut Session, p: &Value) -> Result<Value> {
    let id = strp(p, "material").ok_or_else(|| bad("material.apply", "`material` is required"))?;
    let l = crate::materials::apply_at(s, id, Place::from_params(p))?;
    Ok(json!({ "layer": l.0 }))
}

/// A baked material ball with smart materials on both texture sets
pub fn sample_session(resolution: u32, bake_resolution: u32) -> Result<Session> {
    let mut session = Session::new_project("Material Ball", primitives::material_ball(), "builtin:material_ball", None, resolution)?;
    for set_index in 0..session.set_count() {
        let set = session.doc.project.texture_set_mut(set_index)?;
        set.bake.common.resolution = BakeResolution::from_pixels(bake_resolution).ok_or_else(|| bad("project.sample", "invalid bake resolution"))?;

        // Mesh curvature gives the procedural sample visible edge wear
        set.bake.high_poly.use_low_as_high = true;
        set.bake.maps.curvature.parameters.method = CurvatureMethod::Mesh;
        for ray_settings in
            [&mut set.bake.maps.ao.parameters.rays, &mut set.bake.maps.thickness.parameters.rays, &mut set.bake.maps.bent_normal.parameters.rays]
        {
            ray_settings.rays = 24;
        }
        session.bake_now(set_index, false)?;
    }
    session.active_set = 1;
    crate::materials::apply(&mut session, "iron_rusted")?;
    session.active_set = 0;
    crate::materials::apply(&mut session, "steel_painted_worn")?;

    // Paint above the smart material so new strokes remain visible
    for set_index in (0..session.set_count()).rev() {
        session.active_set = set_index;
        session.selected.remove(&set_index);
        crate::execute(&mut session, "layer.add.paint", &Value::Null)?;
    }
    session.undo.clear();
    session.modified = false;
    Ok(session)
}

fn sample(s: &mut Session, _: &Value) -> Result<Value> {
    let next = sample_session(2048, 1024)?;
    s.switch_to(next);
    Ok(crate::inspect::session(s))
}

pub(crate) fn baked_json(b: Option<&BakedMaps>) -> Value {
    match b {
        Some(b) => json!({ "resolution": b.resolution, "maps": b.maps.keys().map(|m| m.id()).collect::<Vec<_>>() }),
        None => Value::Null,
    }
}
