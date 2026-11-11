//! Color selection effects: the ID colors they select, picked from the ID mesh map

use serde_json::{Value, json};
use texturecraft_core::{BakedMap, EffectKind, LayerId};
use texturecraft_geom::glam::Vec2;
use texturecraft_geom::surface_at;

use crate::commands::{CommandSpec, bad, f32p, has_layer};
use crate::mask_cmds::effect_id;
use crate::tool_cmds::viewport_of;
use crate::{Result, Session, cmd};

/// Most colors one color selection holds; an ID map rarely has more distinct parts
const MAX_SELECTED_COLORS: usize = 256;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "effect.colorSelection.pick",
        "Pick ID color",
        [],
        None,
        "{effect?: a color selection (default: the selected effect), x?, y?: viewport pixels, viewport?: [w,h] | uv?: [u,v] texture set atlas UV (the 2D view), index?: replace this color instead of adding one} → {color: [r,g,b] as the ID map stores it, index} — samples the ID mesh map under the point",
        has_layer,
        pick
    )]
}

/// Applies `effect.set`'s `colors` and `tolerance` params to a color selection
pub(crate) fn apply_params(params: &Value, colors: &mut Vec<[f32; 3]>, tolerance: &mut f32) -> Result<()> {
    if let Some(list) = params.get("colors") {
        *colors = colors_param(list)?;
    }
    if let Some(value) = f32p(params, "tolerance") {
        *tolerance = value.clamp(0.0, 1.0);
    }
    Ok(())
}

/// A non-empty `[[r, g, b], ...]` list of finite values, clamped to 0..1
fn colors_param(list: &Value) -> Result<Vec<[f32; 3]>> {
    const CMD: &str = "effect.set";
    let items = list.as_array().ok_or_else(|| bad(CMD, "`colors` must be a list of [r, g, b]"))?;
    if items.is_empty() || items.len() > MAX_SELECTED_COLORS {
        return Err(bad(CMD, format!("`colors` needs 1 to {MAX_SELECTED_COLORS} colors")));
    }

    items
        .iter()
        .map(|item| match finite_numbers::<3>(item) {
            Some(color) => Ok(color.map(|value| value.clamp(0.0, 1.0) as f32)),
            _ => Err(bad(CMD, "each color must be [r, g, b] with finite numbers")),
        })
        .collect()
}

/// Samples the ID map under a viewport point or at a UV and adds the color to a color selection
fn pick(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "effect.colorSelection.pick";
    let effect = effect_id(session, params, CMD)?;
    let (owner, color_count) = match session.doc.project.effect(effect) {
        Some((owner, found)) => match &found.kind {
            EffectKind::ColorSelection { colors, .. } => (owner, colors.len()),
            other => return Err(bad(CMD, format!("{} is not a color selection", other.name()))),
        },
        None => return Err(bad(CMD, format!("no effect {}", effect.0))),
    };

    let index = params
        .get("index")
        .filter(|index| !index.is_null())
        .map(|index| index.as_u64().ok_or_else(|| bad(CMD, "`index` must be a non-negative integer")))
        .transpose()?;
    let index = match index {
        Some(index) => Some(usize::try_from(index).ok().filter(|index| *index < color_count).ok_or_else(|| bad(CMD, format!("no color {index} to replace")))?),
        None if color_count >= MAX_SELECTED_COLORS => return Err(bad(CMD, format!("a color selection holds at most {MAX_SELECTED_COLORS} colors"))),
        None => None,
    };

    let color = id_color_at(session, params, owner)?;
    session.edit("Pick ID color", |session| {
        let edited = session.doc.project.effect_mut(effect).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?;
        if let EffectKind::ColorSelection { colors, .. } = &mut edited.kind {
            match index.and_then(|index| colors.get_mut(index)) {
                Some(slot) => *slot = color,
                None => colors.push(color),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "color": color, "index": index.unwrap_or(color_count) }))
}

/// The ID map color of the texel a color selection on `owner` reads at the picked point
fn id_color_at(session: &Session, params: &Value, owner: LayerId) -> Result<[f32; 3]> {
    const CMD: &str = "effect.colorSelection.pick";
    let (set_index, _, _) = session.doc.project.locate(owner).ok_or_else(|| bad(CMD, "the effect's layer is not in a texture set"))?;
    let set = session.doc.project.texture_set(set_index)?;
    let baked = session.baked(set_index).filter(|baked| baked.maps.contains_key(&BakedMap::Id));
    let Some(baked) = baked else {
        return Err(bad(CMD, format!("texture set \"{}\" has no ID map: bake one, or set one with textureSet.meshMap.set", set.name)));
    };

    let uv = match params.get("uv") {
        Some(uv) => uv_param(uv)?,
        None => {
            let (Some(x), Some(y)) = (f32p(params, "x"), f32p(params, "y")) else {
                return Err(bad(CMD, "give `x` and `y` (viewport pixels) or `uv`"));
            };

            let (uv, material) = visible_surface(session, Vec2::new(x, y), viewport_of(session, params))
                .ok_or_else(|| bad(CMD, "nothing under that point: pick on the mesh"))?;
            if material != set.material {
                return Err(bad(CMD, "that surface belongs to another texture set"));
            }
            uv
        }
    };

    // The same texel the mask evaluates, so a picked color always selects where it was picked
    let size = set.size();
    let texel = [uv.x * size.0 as f32, uv.y * size.1 as f32];
    if !texel.iter().zip([size.0, size.1]).all(|(position, extent)| (0.0..extent as f32).contains(position)) {
        return Err(bad(CMD, "that point is outside the texture set's UV space"));
    }
    baked.sample_working(BakedMap::Id, texel[0] as u32, texel[1] as u32, size, &set.block_scales()).ok_or_else(|| bad(CMD, "no ID color there"))
}

/// The UV and mesh material under viewport pixel `screen`; hidden texture sets don't stop the ray
fn visible_surface(session: &Session, screen: Vec2, viewport: Vec2) -> Option<(Vec2, usize)> {
    let viewport = viewport.max(Vec2::ONE);
    let device = Vec2::new(screen.x / viewport.x * 2.0 - 1.0, 1.0 - screen.y / viewport.y * 2.0);
    let ray = session.camera.ray(device, viewport.x / viewport.y);
    let sets = &session.doc.project.texture_sets;
    let visible = |triangle: u32| {
        let material = session.mesh.material_of(triangle as usize).unwrap_or(0);
        sets.iter().find(|set| set.material == material).is_none_or(|set| set.visible)
    };

    let hit = session.bvh.closest_filtered(ray.origin, ray.dir, f32::MAX, &visible)?;
    let surface = surface_at(&session.mesh, hit.tri, hit.u, hit.v)?;
    Some((surface.uv, session.mesh.material_of(hit.tri as usize).unwrap_or(0)))
}

/// A `[u, v]` param of finite numbers
fn uv_param(uv: &Value) -> Result<Vec2> {
    match finite_numbers::<2>(uv) {
        Some([u, v]) => Ok(Vec2::new(u as f32, v as f32)),
        _ => Err(bad("effect.colorSelection.pick", "`uv` must be [u, v] with finite numbers")),
    }
}

/// Validate the shape before reading components, without allocating for oversized input
fn finite_numbers<const N: usize>(list: &Value) -> Option<[f64; N]> {
    let items: &[Value; N] = list.as_array()?.as_slice().try_into().ok()?;
    let mut numbers = [0.0; N];
    for (number, item) in numbers.iter_mut().zip(items) {
        *number = item.as_f64().filter(|value| value.is_finite())?;
    }
    Some(numbers)
}
