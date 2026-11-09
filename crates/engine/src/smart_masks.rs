//! Smart masks: procedural mask presets (generators and patterns driven by the baked mesh maps)
//! that replace a layer's mask in one step. Smart materials build their masks from the same
//! presets.

use serde_json::{Value, json};
use texturecraft_core::{
    BlendMode, Effect, EffectKind, Filter, Generator, GeneratorKind, LayerId, Mask, MaskBackground, Procedural, ProceduralKind, Project, Projection, Source,
};

use crate::commands::{CommandSpec, always, bad, has_layer, layer_param, strp};
use crate::session::StackTarget;
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mask.smart.apply", "Apply smart mask", [], None, "{name: smart mask id or label, layer?}", has_layer, apply_cmd),
        cmd!("mask.smart.list", "List smart masks", [], None, "{}", always, |_, _| Ok(json!(
            SMART_MASKS.iter().map(|m| json!({"id": m.id, "label": m.label})).collect::<Vec<_>>()
        ))),
    ]
}

pub struct SmartMask {
    pub id: &'static str,
    pub label: &'static str,
}

pub const SMART_MASKS: &[SmartMask] = &[
    SmartMask { id: "edge_wear", label: "Edges Worn" },
    SmartMask { id: "edge_chipped", label: "Edges Chipped" },
    SmartMask { id: "dirt_cavities", label: "Dirt in Cavities" },
    SmartMask { id: "dust_top", label: "Dust from Top" },
    SmartMask { id: "ground_dirt", label: "Ground Dirt" },
    SmartMask { id: "rust_leaks", label: "Rust Leaks" },
    SmartMask { id: "scratches", label: "Scratches" },
    SmartMask { id: "grunge", label: "Grunge Blotches" },
    SmartMask { id: "paint_chips", label: "Paint Chips" },
    SmartMask { id: "fingerprints", label: "Fingerprints" },
    SmartMask { id: "edges_soft", label: "Edges Soft" },
    SmartMask { id: "edges_sharp", label: "Edges Sharp" },
    SmartMask { id: "exposed", label: "Exposed Areas" },
    SmartMask { id: "thin_areas", label: "Thin Areas" },
    SmartMask { id: "dust_cavities", label: "Dust in Cavities" },
    SmartMask { id: "moss_top", label: "Moss from Top" },
    SmartMask { id: "water_stains", label: "Water Stains" },
    SmartMask { id: "sun_faded", label: "Sun Faded" },
    SmartMask { id: "snow_top", label: "Snow from Top" },
    SmartMask { id: "moss_ground", label: "Moss from Ground" },
    SmartMask { id: "rust_cavities", label: "Rust in Cavities" },
    SmartMask { id: "edge_scratches", label: "Edge Scratches" },
    SmartMask { id: "dirt_streaks", label: "Dirt Streaks" },
    SmartMask { id: "occluded", label: "Occluded Areas" },
];

fn norm(x: &str) -> String {
    x.to_ascii_lowercase().replace([' ', '_', '-'], "")
}

pub fn find(name: &str) -> Option<&'static SmartMask> {
    SMART_MASKS.iter().find(|m| norm(m.id) == norm(name) || norm(m.label) == norm(name))
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// A mesh-map generator with explicit threshold, softness and noise breakup (curvature as baked, without detail sizes).
pub(crate) fn generator(kind: GeneratorKind, level: f32, softness: f32, noise: f32) -> EffectKind {
    EffectKind::Generator(Generator { level, softness, noise_amount: noise, detail: None, ..Generator::new(kind) })
}

/// A direction or position generator (`invert` flips the result).
pub(crate) fn directional(kind: GeneratorKind, direction: [f32; 3], level: f32, softness: f32, noise: f32, invert: bool) -> EffectKind {
    EffectKind::Generator(Generator { level, softness, noise_amount: noise, direction, invert, detail: None, ..Generator::new(kind) })
}

/// A triplanar grayscale pattern fill.
pub(crate) fn pattern(kind: ProceduralKind, scale: f32, seed: u32, contrast: f32) -> EffectKind {
    EffectKind::Fill {
        source: Source::Procedural { pattern: Procedural { kind, scale, seed, contrast, balance: 0.0, invert: false }, low: [0.0; 3], high: [1.0; 3] },
        projection: Projection::Triplanar { scale: 1.0, sharpness: 8.0, offset: [0.0; 3] },
    }
}

/// Background and effects (bottom → top) of preset `id`.
pub(crate) fn preset(id: &str) -> Option<(MaskBackground, Vec<(EffectKind, BlendMode)>)> {
    use BlendMode::*;
    use GeneratorKind as G;
    use ProceduralKind as P;
    let fx = match find(id)?.id {
        "edge_wear" => vec![(generator(G::EdgeWear, 0.55, 0.12, 0.5), Normal)],
        // Wide edge band cut by chips, plus a thin solid line right on the edge.
        "edge_chipped" => vec![
            (generator(G::EdgeWear, 0.8, 0.2, 0.3), Normal),
            (pattern(P::Chips, 3.0, 5, 1.0), Multiply),
            (generator(G::EdgeWear, 0.45, 0.06, 0.6), Lighten),
        ],
        "dirt_cavities" => vec![(generator(G::Dirt, 0.55, 0.25, 0.4), Normal)],
        "dust_top" => vec![(directional(G::Direction, UP, 0.3, 0.15, 0.3, false), Normal), (pattern(P::Clouds, 3.0, 9, 1.4), Multiply)],
        "ground_dirt" => vec![(directional(G::Position, [0.0, -1.0, 0.0], 0.35, 0.25, 0.5, false), Normal), (pattern(P::Grunge, 2.0, 12, 0.8), Multiply)],
        // Streaks on surfaces that don't face up, fed by the crevices they run out of.
        "rust_leaks" => vec![
            (pattern(P::Streaks, 3.0, 4, 1.2), Normal),
            (directional(G::Direction, UP, 0.2, 0.1, 0.0, true), Multiply),
            (generator(G::Dirt, 0.45, 0.2, 0.5), Lighten),
        ],
        "scratches" => vec![(pattern(P::Scratches, 3.0, 6, 1.0), Normal)],
        "grunge" => vec![(pattern(P::Grunge, 2.0, 8, 1.0), Normal)],
        "paint_chips" => vec![(pattern(P::Chips, 5.0, 2, 1.0), Normal), (generator(G::EdgeWear, 0.6, 0.12, 0.5), Lighten)],
        "fingerprints" => vec![(pattern(P::Smudges, 4.0, 3, 1.0), Normal)],
        // A wide, soft highlight on convex edges, and a thin clean line right on them.
        "edges_soft" => vec![(generator(G::EdgeWear, 0.35, 0.35, 0.15), Normal)],
        "edges_sharp" => vec![(generator(G::EdgeWear, 0.65, 0.05, 0.0), Normal)],
        // Open surfaces that light reaches (the opposite of cavities).
        "exposed" => vec![(generator(G::AmbientOcclusion, 0.5, 0.3, 0.0), Normal)],
        "thin_areas" => vec![(generator(G::ThinAreas, 0.3, 0.2, 0.2), Normal)],
        // Cavities filled with soft, cloudy dust.
        "dust_cavities" => vec![(generator(G::Dirt, 0.5, 0.3, 0.2), Normal), (pattern(P::Clouds, 4.0, 21, 1.2), Multiply)],
        // Patches growing on what faces up, broken up by cells.
        "moss_top" => vec![(directional(G::Direction, UP, 0.45, 0.2, 0.6, false), Normal), (pattern(P::Cells, 6.0, 17, 1.4), Multiply)],
        // Tide marks: streaks running down the sides, faded by clouds.
        "water_stains" => vec![
            (pattern(P::Streaks, 2.0, 13, 0.9), Normal),
            (directional(G::Direction, UP, 0.15, 0.2, 0.0, true), Multiply),
            (pattern(P::Clouds, 2.5, 14, 1.0), Multiply),
        ],
        // Surfaces facing the sky, softly graded.
        "sun_faded" => vec![(directional(G::Direction, UP, 0.5, 0.45, 0.1, false), Normal)],
        // A crisp cap on what faces up, with a ragged border.
        "snow_top" => vec![(directional(G::Direction, UP, 0.6, 0.08, 0.6, false), Normal)],
        // Growth creeping up from the lowest parts, in clumps.
        "moss_ground" => vec![(directional(G::Position, [0.0, -1.0, 0.0], 0.4, 0.3, 0.5, false), Normal), (pattern(P::Cells, 6.0, 25, 1.4), Multiply)],
        // Cavities eaten by blotchy corrosion.
        "rust_cavities" => vec![(generator(G::Dirt, 0.6, 0.25, 0.5), Normal), (pattern(P::Grunge, 3.0, 27, 1.3), Multiply)],
        // Scratches gathered where edges get handled.
        "edge_scratches" => vec![(generator(G::EdgeWear, 0.75, 0.3, 0.2), Normal), (pattern(P::Scratches, 4.0, 29, 1.2), Multiply)],
        // Grime running down from the crevices it collects in.
        "dirt_streaks" => vec![(pattern(P::Streaks, 2.5, 31, 1.1), Normal), (generator(G::Dirt, 0.5, 0.25, 0.4), Multiply)],
        // Hidden surfaces light hardly reaches (the opposite of exposed areas).
        "occluded" => vec![(directional(G::AmbientOcclusion, UP, 0.5, 0.3, 0.0, true), Normal)],
        _ => return None,
    };
    Some((MaskBackground::Black, fx))
}

/// Build a mask from a background and effects, allocating effect ids in `p`.
pub(crate) fn build_mask(p: &mut Project, bg: MaskBackground, effects: Vec<(EffectKind, BlendMode)>) -> Mask {
    let mut m = Mask::new(bg);
    for (k, blend) in effects {
        m.effects.push(Effect { blend, ..Effect::new(p.alloc_id(), k) });
    }
    m
}

/// Preset `id` as a mask, optionally inverted (e.g. paint that survives where wear is not).
pub(crate) fn mask(p: &mut Project, id: &str, invert: bool) -> Result<Mask> {
    let (bg, mut fx) = preset(id).ok_or_else(|| EngineError::Other(format!("unknown smart mask `{id}`")))?;
    if invert {
        fx.push((EffectKind::Filter(Filter::Invert), BlendMode::Normal));
    }
    Ok(build_mask(p, bg, fx))
}

/// Replace layer `layer`'s mask with smart mask `name` (one undo step).
pub fn apply(s: &mut Session, layer: LayerId, name: &str) -> Result<()> {
    let sm = find(name).ok_or_else(|| EngineError::Other(format!("unknown smart mask `{name}`")))?;
    s.edit(&format!("Apply smart mask {}", sm.label), |s| {
        let m = mask(&mut s.doc.project, sm.id, false)?;
        let old = s.doc.project.layer_mut(layer)?.mask.replace(m);
        for e in old.map(|m| m.effects).unwrap_or_default() {
            s.doc.pixels.masks.remove(&e.id);
        }
        s.target = StackTarget::Mask;
        Ok(())
    })
}

fn apply_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let name = strp(p, "name").ok_or_else(|| bad("mask.smart.apply", "`name` is required"))?;
    let layer = layer_param(s, "mask.smart.apply", p)?;
    apply(s, layer, name)?;
    Ok(json!({ "layer": layer.0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_resolves_and_ids_are_unique() {
        let (mut ids, mut labels) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        for m in SMART_MASKS {
            assert!(preset(m.id).is_some(), "{}", m.id);
            assert!(ids.insert(norm(m.id)) && labels.insert(norm(m.label)), "duplicate {}", m.id);
            assert_eq!(find(m.label).map(|f| f.id), Some(m.id));
        }
    }
}
