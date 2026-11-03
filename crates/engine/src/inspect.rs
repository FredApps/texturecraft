//! JSON views of the session for agents: everything an agent changed must be visible here
//! without a screenshot.

use serde_json::{Value, json};
use texturecraft_core::{Effect, EffectKind, Fill, Filter, Layer, LayerKind, NormalFormat, Projection, Source};

use crate::Session;
use crate::session::StackTarget;

fn source_json(s: &Source) -> Value {
    match s {
        Source::Uniform(v) => json!({ "uniform": v }),
        Source::Procedural { pattern, low, high } => json!({
            "procedural": pattern.kind.label(), "scale": pattern.scale, "seed": pattern.seed, "contrast": pattern.contrast, "low": low, "high": high
        }),
        Source::Image { name, normal_format: NormalFormat::OpenGl } => json!({ "image": name }),
        Source::Image { name, normal_format: NormalFormat::DirectX } => json!({ "image": name, "normalFormat": "directX" }),
        Source::Anchor { anchor, channel } => json!({ "anchor": anchor.0, "channel": channel.id() }),
    }
}

fn fill_json(f: &Fill) -> Value {
    let mut v = json!({
        "projection": f.projection.label(),
        "sources": f.sources.iter().map(|(c, s)| (c.id().to_string(), source_json(s))).collect::<serde_json::Map<_, _>>(),
    });
    if let Projection::Decal(d) = &f.projection {
        v["decal"] = json!(d);
    }
    if let Some(link) = &f.link {
        v["link"] = json!(link);
    }
    v
}

/// A mask or content (`content`) effect.
fn effect_json(s: &Session, e: &Effect, content: bool) -> Value {
    let mut ej = json!({ "id": e.id.0, "name": e.name, "kind": e.kind.name(), "visible": e.visible, "blend": e.blend.label(), "opacity": e.opacity });
    match &e.kind {
        EffectKind::Generator(g) => {
            ej["generator"] = json!({ "type": format!("{:?}", g.kind), "level": g.level, "softness": g.softness, "noise": g.noise_amount, "offset": g.offset, "invert": g.invert })
        }
        EffectKind::Levels(lv) => ej["levels"] = json!([lv.in_low, lv.in_high, lv.gamma, lv.out_low, lv.out_high]),
        EffectKind::Path(paths) => {
            ej["paths"] = json!(paths.iter().map(|p| json!({ "points": p.points.len(), "uv": p.uv, "closed": p.closed, "fill": p.fill })).collect::<Vec<_>>());
            ej["tiles"] = json!(if content {
                s.doc.pixels.layers.iter().filter(|((id, _), _)| *id == e.id).map(|(_, img)| img.allocated_count()).sum::<usize>()
            } else {
                s.doc.pixels.masks.get(&e.id).map(|i| i.allocated_count()).unwrap_or(0)
            });
        }
        EffectKind::Paint if content => {
            ej["tiles"] = json!(s.doc.pixels.layers.iter().filter(|((id, _), _)| *id == e.id).map(|(_, img)| img.allocated_count()).sum::<usize>())
        }
        EffectKind::Paint => ej["tiles"] = json!(s.doc.pixels.masks.get(&e.id).map(|i| i.allocated_count()).unwrap_or(0)),
        EffectKind::Fill { source, .. } => ej["source"] = source_json(source),
        EffectKind::Material(f) => ej["fill"] = fill_json(f),
        EffectKind::Filter(f) => {
            ej["filter"] = json!(f);
            match f {
                Filter::Hsl { hue, saturation, lightness } => ej["hsl"] = json!([hue, saturation, lightness]),
                Filter::Blur { radius } => ej["radius"] = json!(radius),
                _ => {}
            }
        }
        _ => {}
    }
    ej
}

fn layer_json(s: &Session, l: &Layer, depth: usize) -> Value {
    let channels: serde_json::Map<String, Value> =
        l.channels.iter().map(|(c, b)| (c.id().to_string(), json!({ "mode": b.mode.label(), "opacity": b.opacity }))).collect();
    let mut v = json!({
        "id": l.id.0,
        "name": l.name,
        "kind": l.kind.name(),
        "visible": l.visible,
        "channels": channels,
        "colorTag": l.color_tag,
    });
    if let LayerKind::Fill(f) = &l.kind {
        v["fill"] = fill_json(f);
    }
    if let Some(src) = l.instance_source() {
        let source = s.doc.project.layers.get(&src);
        let set = s.doc.project.locate(src).map(|(ts, _, _)| ts);
        v["instanceOf"] = json!({ "layer": src.0, "name": source.map(|x| x.name.as_str()), "textureSet": set });
    }
    if l.height_mask.enabled {
        v["heightMask"] = json!({ "contrast": l.height_mask.contrast, "offset": l.height_mask.offset });
    }
    if let Some(parts) = &l.geometry_mask {
        v["geometryMask"] = json!(parts);
    }
    if matches!(l.kind, LayerKind::Paint) {
        // Sorted: the store is a hash map, and inspect output must be stable.
        let mut painted: Vec<(&str, usize)> =
            s.doc.pixels.layers.iter().filter(|((id, _), _)| *id == l.id).map(|((_, c), img)| (c.id(), img.allocated_count())).collect();
        painted.sort_unstable();
        v["painted"] = json!(painted.iter().map(|(c, n)| json!({ "channel": c, "tiles": n })).collect::<Vec<_>>());
    }
    if let Some(m) = &l.mask {
        v["mask"] = json!({
            "enabled": m.enabled,
            "background": format!("{:?}", m.background).to_lowercase(),
            "effects": m.effects.iter().map(|e| effect_json(s, e, false)).collect::<Vec<_>>(),
        });
    }
    if !l.content_effects.is_empty() {
        v["contentEffects"] = json!(l.content_effects.iter().map(|e| effect_json(s, e, true)).collect::<Vec<_>>());
    }
    if depth < texturecraft_core::project::MAX_DEPTH {
        let kids: Vec<Value> = l.children().iter().rev().filter_map(|c| s.doc.project.layers.get(c)).map(|c| layer_json(s, c, depth + 1)).collect();
        if l.is_group() {
            v["children"] = json!(kids);
        }
    }
    v
}

/// The layer stack of texture set `ts`, top → bottom (as displayed).
pub fn stack(s: &Session, ts: usize) -> Value {
    let Some(set) = s.doc.project.texture_sets.get(ts) else { return Value::Null };
    json!(set.stack.iter().rev().filter_map(|id| s.doc.project.layers.get(id)).map(|l| layer_json(s, l, 0)).collect::<Vec<_>>())
}

pub fn texture_sets(s: &Session) -> Value {
    json!((0..s.set_count()).map(|i| texture_set(s, i)).collect::<Vec<_>>())
}

/// One texture set; UV-tiled sets list their tiles and where each sits in the texel atlas.
pub fn texture_set(s: &Session, i: usize) -> Value {
    let Some(t) = s.doc.project.texture_sets.get(i) else { return Value::Null };
    let grid = t.tile_grid();
    let tiles: Vec<Value> =
        t.uv_tiles.iter().filter_map(|u| grid?.block(*u).map(|(x, y)| json!({ "udim": u, "block": [x, y], "resolution": t.tile_resolution(*u) }))).collect();
    json!({
        "index": i,
        "name": t.name,
        "resolution": t.resolution,
        "size": t.size(),
        "blockResolution": t.block_resolution(),
        "uvTiles": t.uv_tiles,
        "tiles": tiles,
        "channels": t.channels.iter().map(|(c, f)| json!({ "channel": c.id(), "format": f.label() })).collect::<Vec<_>>(),
        "layers": s.doc.project.walk(i).len(),
        "parts": crate::set_cmds::parts(s, i),
        "baked": crate::project_cmds::baked_json(s.doc.baked.get(&i)),
        "visible": t.visible,
        "active": i == s.ts(),
    })
}

/// The whole session.
pub fn session(s: &Session) -> Value {
    let ts = s.ts();
    json!({
        "project": s.doc.project.name,
        "mesh": { "source": s.doc.project.mesh, "triangles": s.mesh.triangle_count(), "vertices": s.mesh.vertex_count(), "materials": s.mesh.materials },
        "path": s.path,
        "modified": s.modified,
        "textureSets": texture_sets(s),
        "bakeMeshes": crate::bake_cmds::meshes_json(s),
        "activeTextureSet": ts,
        "selectedLayer": s.selected_layer().map(|l| l.0),
        "target": match s.target { StackTarget::Content => json!("content"), StackTarget::Mask => json!("mask"), StackTarget::Effect(e) => json!(e.0) },
        "layers": stack(s, ts),
        "tool": { "tool": s.tool.tool.id(), "brush": crate::tool_cmds::brush_json(s) },
        "view": s.view.label(),
        "camera": crate::view_cmds::camera_json(s),
        "display": s.doc.project.display,
        "history": { "undo": s.undo_label(), "redo": s.redo_label(), "depth": s.history_labels().len() },
        "memoryBytes": s.memory().total(),
        "pendingTiles": s.pending_tiles(),
    })
}
