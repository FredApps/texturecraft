use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, boolp, u64p};
use crate::set_cmds::ts_param;
use crate::{Result, Session, cmd};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "bake.tile.set",
        "Include UV tile in bake",
        [],
        None,
        "{ts?, tile: UDIM number, enabled: bool} → {ts, tile, enabled}; excluded tiles keep existing baked pixels",
        always,
        set_enabled
    )]
}

fn set_enabled(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "bake.tile.set";
    let texture_set = ts_param(session, params, COMMAND)?;
    let tile = u64p(params, "tile").and_then(|tile| u32::try_from(tile).ok()).ok_or_else(|| bad(COMMAND, "`tile` must be a UDIM number"))?;
    let enabled = boolp(params, "enabled").ok_or_else(|| bad(COMMAND, "`enabled` must be true or false"))?;
    let settings = session.doc.project.texture_set(texture_set)?;
    if !settings.uv_tiles.contains(&tile) {
        return Err(bad(COMMAND, format!("texture set {} has no UV tile {tile}", settings.name)));
    }

    let was_enabled = !settings.bake_excluded_tiles.contains(&tile);
    if enabled != was_enabled {
        session.edit("Include UV tile in bake", |session| {
            let settings = session.doc.project.texture_set_mut(texture_set)?;
            if enabled {
                settings.bake_excluded_tiles.remove(&tile);
            } else {
                settings.bake_excluded_tiles.insert(tile);
            }
            Ok(())
        })?;
    }
    Ok(json!({"ts": texture_set, "tile": tile, "enabled": enabled}))
}
