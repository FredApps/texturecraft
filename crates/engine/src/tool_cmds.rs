//! Tool commands: pick a tool, set brush and material values, and paint scripted strokes (the
//! way agents and the CLI paint).

use serde_json::{Value, json};
use texturecraft_core::Channel;
use texturecraft_geom::glam::Vec2;
use texturecraft_paint::{Alignment, BrushAlpha, alphas};

use crate::commands::{CommandSpec, always, bad, boolp, channel_param, color_param, f32p, strp};
use crate::fill_hit_test::FillHitTest;
use crate::fill_mode::FillMode;
use crate::session::{CloneFrom, CloneSource, StackTarget, Tool};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("tool.paint", "Paint", ["Tools"], Some("1"), "{}", always, |s, _| set_tool(s, Tool::Paint)),
        cmd!("tool.eraser", "Eraser", ["Tools"], Some("2"), "{}", always, |s, _| set_tool(s, Tool::Eraser)),
        cmd!("tool.projection", "Projection", ["Tools"], Some("3"), "{}", always, |s, _| set_tool(s, Tool::Projection)),
        cmd!("tool.polygonFill", "Polygon fill", ["Tools"], Some("4"), "{}", always, |s, _| set_tool(s, Tool::PolygonFill)),
        cmd!("tool.smudge", "Smudge", ["Tools"], Some("5"), "{}", always, |s, _| set_tool(s, Tool::Smudge)),
        cmd!("tool.clone", "Clone", ["Tools"], Some("6"), "{}", always, |s, _| set_tool(s, Tool::Clone)),
        cmd!("tool.colorPicker", "Color picker", ["Tools"], Some("P"), "{}", always, |s, _| set_tool(s, Tool::ColorPicker)),
        cmd!(
            "tool.geometryMask",
            "Geometry mask",
            ["Tools"],
            Some("8"),
            "{} — then tool.click toggles the selected layer on the mesh part under the point",
            always,
            |s, _| { set_tool(s, Tool::GeometryMask) }
        ),
        cmd!("tool.select", "Select tool", [], None, "{tool: paint|eraser|projection|polygonFill|smudge|clone|colorPicker|path}", always, select),
        cmd!(
            "tool.brush",
            "Brush settings",
            [],
            None,
            "{size?: 0..100, flow?: 0..100, opacity?: 0..100, spacing?: 1..1000, hardness?: 0..100, angle?, followPath?, alignment?: tangent|camera, lazy?, jitterSize?, jitterAngle?, jitterFlow?, jitterPosition?, alpha?: procedural name | {image: resource name} | {library: id (alpha.list)}, backfaceCulling?, symmetryX?, symmetryY?, symmetryZ?, symmetryRadial?: 0..32 copies around symmetryAxis?: x|y|z (default y), minSize?, smudgeStrength?: 0..100, cloneSource?: composite|layer (what the Clone tool copies), particles?: {enabled?, count?: 1..16, lifetime?: 1..128 steps, gravity?: 0..2}}",
            always,
            brush
        ),
        cmd!("alpha.list", "List brush alphas", [], None, "{} — procedural alphas and the built-in library", always, |_, _| Ok(json!({
            "procedural": BrushAlpha::ALL.map(BrushAlpha::label),
            "library": alphas::library().iter().map(|a| json!({"id": a.id, "label": a.label, "category": a.category})).collect::<Vec<_>>(),
        }))),
        cmd!(
            "tool.material",
            "Tool material",
            [],
            None,
            "{channel, color?|value?, enabled?, solo?: true (paint this channel only)} or {maskValue?: 0..1}",
            always,
            material
        ),
        cmd!(
            "tool.swapMaskValue",
            "Invert mask value",
            ["Tools"],
            Some("X"),
            "{} — mask value becomes 1 − value: switch the brush between adding to and removing from the mask",
            paints_mask,
            swap_mask_value
        ),
        cmd!(
            "tool.stencil",
            "Projection stencil",
            [],
            None,
            "{image?: resource name, center?: [x,y] NDC, scale?: 0.01..10, rotation?: degrees, tile?, opacity?, clear?: bool}",
            always,
            stencil
        ),
        cmd!(
            "tool.cloneSource",
            "Set clone source",
            [],
            None,
            "{x, y, viewport?: [w,h]} a point on the mesh, or {uv: [u,v]}, or {clear: true} — the Clone tool copies from here (V + click)",
            always,
            clone_source
        ),
        cmd!("tool.fillMode", "Polygon fill mode", [], None, "{mode: triangle|polygon|uvIsland|mesh}", always, fill_mode),
        cmd!(
            "tool.fillSelection",
            "Polygon fill selection",
            [],
            None,
            "{hitTest?: intersection|containedCenter (a box selects polygons it touches, or whose center it holds), visibleOnly?: bool (3D view boxes select only what the camera sees)}",
            always,
            fill_selection
        ),
        cmd!("tool.stroke", "Paint stroke", [], None, "{points: [[x,y],...] viewport pixels, viewport?: [w,h]} — paints with the current tool", always, stroke),
        cmd!(
            "tool.strokeUv",
            "Paint stroke in the 2D view",
            [],
            None,
            "{points: [[u,v],...] texture coordinates 0..1} — paints in UV space like the 2D view",
            always,
            stroke_uv
        ),
        cmd!(
            "tool.click",
            "Tool click",
            [],
            None,
            "{x, y, viewport?: [w,h]} — polygon fill / color picker / single dab; polygon fill also takes a box {x0, y0, x1, y1, viewport?}, a 2D view point {uv: [u,v]} or a 2D view box {uv0: [u,v], uv1: [u,v]}",
            always,
            click
        ),
        cmd!("tool.state", "Tool state", [], None, "{}", always, |s, _| Ok(serde_json::to_value(&s.tool).unwrap_or(Value::Null))),
    ]
}

pub(crate) fn set_tool(session: &mut Session, tool: Tool) -> Result<Value> {
    session.cancel_stroke();
    // A box fill under way keeps what it filled as one undo step
    session.end_polygon_fill_box();
    session.tool.tool = tool;
    session.revision = session.revision.wrapping_add(1);
    Ok(json!({ "tool": tool.id() }))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let name = strp(p, "tool").ok_or_else(|| bad("tool.select", "`tool` is required"))?;
    let t = Tool::parse(name).ok_or_else(|| bad("tool.select", format!("unknown tool `{name}`")))?;
    set_tool(s, t)
}

/// Brush values use the UI's 0–100 scales, so agents and the sliders speak the same numbers.
fn brush(s: &mut Session, p: &Value) -> Result<Value> {
    let b = &mut s.tool.brush;
    let pct = |k: &str| f32p(p, k).map(|v| v / 100.0);
    if let Some(v) = pct("size") {
        b.size = v * 0.25;
    }
    if let Some(v) = pct("minSize") {
        b.min_size = v;
    }
    if let Some(v) = pct("flow") {
        b.flow = v;
    }
    if let Some(v) = pct("opacity") {
        b.opacity = v;
    }
    if let Some(v) = pct("spacing") {
        b.spacing = v;
    }
    if let Some(v) = pct("hardness") {
        b.hardness = v;
    }
    if let Some(v) = f32p(p, "angle") {
        b.angle = v;
    }
    if let Some(v) = boolp(p, "followPath") {
        b.follow_path = v;
    }
    if let Some(a) = strp(p, "alignment") {
        b.alignment = match a {
            "camera" => Alignment::Camera,
            "tangent" | "uv" => Alignment::Tangent,
            other => return Err(bad("tool.brush", format!("unknown alignment `{other}`"))),
        };
    }
    if let Some(v) = f32p(p, "lazy") {
        b.lazy = v;
    }
    for (k, slot) in [("jitterSize", &mut b.jitter_size), ("jitterAngle", &mut b.jitter_angle), ("jitterFlow", &mut b.jitter_flow)] {
        if let Some(v) = f32p(p, k) {
            *slot = v / 100.0;
        }
    }
    if let Some(v) = f32p(p, "jitterPosition") {
        b.jitter_position = v / 100.0;
    }
    let alpha = p.get("alpha");
    if let Some(a) = strp(p, "alpha") {
        b.alpha = BrushAlpha::parse(a).ok_or_else(|| bad("tool.brush", format!("unknown alpha `{a}`")))?;
        (b.alpha_image, b.alpha_library) = (None, None);
    } else if let Some(name) = alpha.and_then(|a| a.get("image")).and_then(Value::as_str) {
        if !s.images.contains_key(name) {
            return Err(bad("tool.brush", format!("no image resource `{name}` (import it first)")));
        }
        (b.alpha_image, b.alpha_library) = (Some(name.to_string()), None);
    } else if let Some(id) = alpha.and_then(|a| a.get("library")).and_then(Value::as_str) {
        let a = alphas::find(id).ok_or_else(|| bad("tool.brush", format!("unknown library alpha `{id}` (see alpha.list)")))?;
        // Render it now rather than on the first stamp.
        a.image();
        (b.alpha_image, b.alpha_library) = (None, Some(a.id.clone()));
    }
    if let Some(v) = boolp(p, "backfaceCulling") {
        b.backface_culling = v;
    }
    for (k, slot) in [("symmetryX", &mut b.symmetry.x), ("symmetryY", &mut b.symmetry.y), ("symmetryZ", &mut b.symmetry.z)] {
        if let Some(v) = boolp(p, k) {
            *slot = v;
        }
    }
    if let Some(v) = f32p(p, "symmetryRadial") {
        b.symmetry.radial = v.clamp(0.0, 32.0) as u8;
    }
    if let Some(a) = strp(p, "symmetryAxis") {
        b.symmetry.axis = match a.to_ascii_lowercase().as_str() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => return Err(bad("tool.brush", format!("unknown symmetry axis `{a}` (x, y or z)"))),
        };
    }
    if let Some(v) = boolp(p, "screenSpace") {
        b.screen_space = v;
    }
    if let Some(q) = p.get("particles") {
        let pt = &mut b.particles;
        pt.enabled = boolp(q, "enabled").unwrap_or(pt.enabled);
        pt.count = f32p(q, "count").map_or(pt.count, |v| v as u32);
        pt.lifetime = f32p(q, "lifetime").map_or(pt.lifetime, |v| v as u32);
        pt.gravity = f32p(q, "gravity").unwrap_or(pt.gravity);
    }
    *b = b.sanitized();
    if let Some(v) = f32p(p, "smudgeStrength") {
        s.tool.smudge_strength = (v / 100.0).clamp(0.0, 1.0);
    }
    if let Some(v) = strp(p, "cloneSource") {
        s.tool.clone_from = match v {
            "composite" => CloneFrom::Composite,
            "layer" => CloneFrom::Layer,
            other => return Err(bad("tool.brush", format!("unknown clone source `{other}`: use composite or layer"))),
        };
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(brush_json(s))
}

pub(crate) fn brush_json(s: &Session) -> Value {
    let b = &s.tool.brush;
    json!({
        "size": b.size / 0.25 * 100.0, "minSize": b.min_size * 100.0, "flow": b.flow * 100.0, "opacity": b.opacity * 100.0,
        "spacing": b.spacing * 100.0, "hardness": b.hardness * 100.0, "angle": b.angle, "followPath": b.follow_path,
        "alignment": format!("{:?}", b.alignment).to_lowercase(), "lazy": b.lazy,
        "alpha": match (&b.alpha_image, &b.alpha_library) {
            (Some(name), _) => json!({ "image": name }),
            (None, Some(id)) => json!({ "library": id }),
            (None, None) => json!(b.alpha.label()),
        },
        "backfaceCulling": b.backface_culling, "symmetry": [b.symmetry.x, b.symmetry.y, b.symmetry.z],
        "radialSymmetry": { "copies": b.symmetry.radial, "axis": (["x", "y", "z"].get(b.symmetry.axis as usize)) },
        "smudgeStrength": s.tool.smudge_strength * 100.0, "particles": b.particles,
        "cloneSource": if s.tool.clone_from == CloneFrom::Layer { "layer" } else { "composite" },
    })
}

fn material(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(v) = f32p(p, "maskValue") {
        s.tool.mask_value = v.clamp(0.0, 1.0);
    }
    if let Some(c) = channel_param("tool.material", p, "channel")? {
        if let Some(v) = color_param(p, "color").or_else(|| color_param(p, "value")) {
            let v = if c == Channel::Height { v.map(|x| x.clamp(-1.0, 1.0)) } else { v.map(|x| x.clamp(0.0, 1.0)) };
            s.tool.values.insert(c, v);
            s.tool.enabled.insert(c);
        }
        match boolp(p, "enabled") {
            Some(true) => {
                s.tool.enabled.insert(c);
            }
            Some(false) => {
                s.tool.enabled.remove(&c);
            }
            None => {}
        }
        if boolp(p, "solo") == Some(true) {
            s.tool.enabled = std::iter::once(c).collect();
        }
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({
        "values": s.tool.values.iter().map(|(c, v)| (c.id().to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
        "enabled": s.tool.enabled.iter().map(|c| c.id()).collect::<Vec<_>>(),
        "maskValue": s.tool.mask_value,
    }))
}

/// Whether strokes paint the selected layer's mask: the mask itself or one of its effects
fn paints_mask(session: &Session) -> std::result::Result<(), String> {
    session.selected_layer().ok_or("no layer selected")?;

    let project = &session.doc.project;
    let targets_mask = match session.target {
        StackTarget::Content => false,
        StackTarget::Mask => true,
        StackTarget::Effect(effect_id) => project
            .effect(effect_id)
            .and_then(|(owner, _)| project.layers.get(&owner))
            .is_some_and(|owner_layer| !owner_layer.content_effects.iter().any(|effect| effect.id == effect_id)),
    };

    if targets_mask { Ok(()) } else { Err("select a mask to paint on".into()) }
}

fn swap_mask_value(session: &mut Session, _: &Value) -> Result<Value> {
    session.tool.mask_value = 1.0 - session.tool.mask_value;
    session.revision = session.revision.wrapping_add(1);

    Ok(json!({ "maskValue": session.tool.mask_value }))
}

fn stencil(s: &mut Session, p: &Value) -> Result<Value> {
    if boolp(p, "clear") == Some(true) {
        s.tool.stencil = None;
        return Ok(Value::Null);
    }
    if let Some(name) = strp(p, "image") {
        if !s.doc.resources.contains_key(name) {
            return Err(bad("tool.stencil", format!("no resource `{name}` (import it first)")));
        }
        let keep = s.tool.stencil.take();
        let mut st = crate::session::StencilState::new(name);
        if let Some(k) = keep {
            st = crate::session::StencilState { image: name.to_string(), ..k };
        }
        s.tool.stencil = Some(st);
    }
    let st = s.tool.stencil.as_mut().ok_or_else(|| bad("tool.stencil", "no stencil: give `image`"))?;
    if let Some(c) = color_param(p, "center").map(|c| [c[0], c[1]]).or_else(|| pair(p, "center")) {
        st.center = c.map(|v| v.clamp(-4.0, 4.0));
    }
    if let Some(v) = f32p(p, "scale") {
        st.scale = v.clamp(0.01, 10.0);
    }
    if let Some(v) = f32p(p, "rotation") {
        st.rotation = v.rem_euclid(360.0);
    }
    if let Some(v) = boolp(p, "tile") {
        st.tile = v;
    }
    if let Some(v) = f32p(p, "opacity") {
        st.opacity = v.clamp(0.0, 1.0);
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(serde_json::to_value(&s.tool.stencil).unwrap_or(Value::Null))
}

/// A `[a, b]` param of two finite numbers.
fn pair(p: &Value, key: &str) -> Option<[f32; 2]> {
    let a = p.get(key)?.as_array()?;
    let v: Vec<f32> = a.iter().filter_map(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite()).collect();
    match v.as_slice() {
        [x, y] => Some([*x, *y]),
        _ => None,
    }
}

fn clone_source(s: &mut Session, p: &Value) -> Result<Value> {
    if boolp(p, "clear") == Some(true) {
        s.tool.clone_source = None;
        return Ok(Value::Null);
    }
    let uv = match (pair(p, "uv"), f32p(p, "x"), f32p(p, "y")) {
        (Some(uv), _, _) => uv.map(|v| v.clamp(0.0, 1.0)),
        (None, Some(x), Some(y)) => {
            let hit = s.pick(Vec2::new(x, y), viewport_of(s, p)).ok_or_else(|| bad("tool.cloneSource", "nothing under that point"))?;
            if s.doc.project.texture_set(s.ts()).ok().map(|t| t.material) != Some(hit.material) {
                return Err(bad("tool.cloneSource", "that surface belongs to another texture set"));
            }
            [hit.uv.x, hit.uv.y]
        }
        _ => return Err(bad("tool.cloneSource", "give `uv: [u, v]` or `x` and `y`")),
    };
    s.tool.clone_source = Some(CloneSource { uv, offset: None });
    s.revision = s.revision.wrapping_add(1);
    Ok(serde_json::to_value(s.tool.clone_source).unwrap_or(Value::Null))
}

fn fill_mode(session: &mut Session, params: &Value) -> Result<Value> {
    session.tool.fill_mode = match strp(params, "mode").unwrap_or("triangle") {
        "triangle" => FillMode::Triangle,
        "polygon" => FillMode::Polygon,
        "uvIsland" | "island" => FillMode::UvIsland,
        "mesh" => FillMode::Mesh,
        other => return Err(bad("tool.fillMode", format!("unknown fill mode `{other}`"))),
    };

    Ok(Value::Null)
}

fn fill_selection(session: &mut Session, params: &Value) -> Result<Value> {
    match strp(params, "hitTest") {
        None => {}
        Some("intersection") => session.tool.fill_hit_test = FillHitTest::Intersection,
        Some("containedCenter") => session.tool.fill_hit_test = FillHitTest::ContainedCenter,
        Some(other) => return Err(bad("tool.fillSelection", format!("unknown hit-test `{other}` (intersection or containedCenter)"))),
    }
    if let Some(visible_only) = boolp(params, "visibleOnly") {
        session.tool.fill_visible_only = visible_only;
    }

    Ok(Value::Null)
}

pub(crate) fn viewport_of(s: &Session, p: &Value) -> Vec2 {
    match p.get("viewport").and_then(Value::as_array) {
        Some(a) if a.len() == 2 => {
            let w = a[0].as_f64().unwrap_or(0.0) as f32;
            let h = a[1].as_f64().unwrap_or(0.0) as f32;
            if w.is_finite() && h.is_finite() && w >= 16.0 && h >= 16.0 { Vec2::new(w.min(16384.0), h.min(16384.0)) } else { s.viewport }
        }
        _ => s.viewport,
    }
}

/// A `[[x, y], ...]` param of at most 100000 finite points, None when the key is absent.
pub(crate) fn points_param(cmd: &str, p: &Value, key: &str) -> Result<Option<Vec<Vec2>>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let pts = v.as_array().ok_or_else(|| bad(cmd, format!("`{key}` must be an array of [x, y]")))?;
    if pts.len() > 100_000 {
        return Err(bad(cmd, "at most 100000 points"));
    }
    let points: Vec<Vec2> = pts
        .iter()
        .filter_map(|v| {
            let a = v.as_array()?;
            let (x, y) = (a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
            (x.is_finite() && y.is_finite()).then_some(Vec2::new(x, y))
        })
        .collect();
    if points.is_empty() {
        return Err(bad(cmd, "no valid points"));
    }
    Ok(Some(points))
}

fn stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let points = points_param("tool.stroke", p, "points")?.ok_or_else(|| bad("tool.stroke", "`points` must be an array of [x, y]"))?;
    let vp = viewport_of(s, p);
    let stamps = s.stroke_points(&points, vp)?;
    s.update();
    Ok(json!({ "stamps": stamps }))
}

fn click(session: &mut Session, params: &Value) -> Result<Value> {
    if session.tool.tool == Tool::PolygonFill
        && let Some(triangles) = polygon_fill_click(session, params)?
    {
        session.update();
        return Ok(json!({ "triangles": triangles }));
    }

    let (Some(x), Some(y)) = (f32p(params, "x"), f32p(params, "y")) else { return Err(bad("tool.click", "`x` and `y` are required")) };

    let point = Vec2::new(x, y);
    let viewport = viewport_of(session, params);
    let out = match session.tool.tool {
        Tool::PolygonFill => json!({ "triangles": session.polygon_fill(point, viewport)? }),
        Tool::ColorPicker => {
            let picked = session.color_pick(point, viewport)?;
            json!(picked.iter().map(|(channel, value)| (channel.id().to_string(), json!(value))).collect::<serde_json::Map<_, _>>())
        }
        Tool::GeometryMask => crate::instance_cmds::toggle_part_at(session, point, viewport)?,
        _ => json!({ "stamps": session.stroke_points(&[point], viewport)? }),
    };
    session.update();
    Ok(out)
}

/// The polygon fill forms of `tool.click` besides a plain `{x, y}` point: None for that point
fn polygon_fill_click(session: &mut Session, params: &Value) -> Result<Option<usize>> {
    let screen_box = (f32p(params, "x0"), f32p(params, "y0"), f32p(params, "x1"), f32p(params, "y1"));
    let given = |keys: &[&str]| keys.iter().any(|key| params.get(*key).is_some());
    let filled = match (pair(params, "uv0"), pair(params, "uv1"), pair(params, "uv"), screen_box) {
        (Some(corner), Some(opposite), ..) => session.polygon_fill_box_uv(corner.into(), opposite.into())?,
        _ if given(&["uv0", "uv1"]) => return Err(bad("tool.click", "a UV box needs `uv0` and `uv1` as [u, v]")),
        (_, _, Some(uv), _) => session.polygon_fill_uv(uv.into())?,
        _ if given(&["uv"]) => return Err(bad("tool.click", "a 2D view point needs `uv` as [u, v]")),
        (.., (Some(x0), Some(y0), Some(x1), Some(y1))) => session.polygon_fill_box(Vec2::new(x0, y0), Vec2::new(x1, y1), viewport_of(session, params))?,
        _ if given(&["x0", "y0", "x1", "y1"]) => return Err(bad("tool.click", "a screen box needs numbers `x0`, `y0`, `x1` and `y1`")),
        _ => return Ok(None),
    };

    Ok(Some(filled))
}

fn stroke_uv(s: &mut Session, p: &Value) -> Result<Value> {
    let points = points_param("tool.strokeUv", p, "points")?.ok_or_else(|| bad("tool.strokeUv", "`points` must be an array of [u, v]"))?;
    let tiles = s.stroke_uv_points(&points)?;
    s.update();
    Ok(json!({ "tiles": tiles }))
}
