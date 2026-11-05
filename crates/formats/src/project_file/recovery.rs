use serde::Deserialize;
use serde_json::Value;
use texturecraft_core::{BakeSettings, BakedMap, Project};

pub(super) fn read_project(json: &[u8]) -> serde_json::Result<(Project, bool)> {
    let error = match serde_json::from_slice(json) {
        Ok(project) => return Ok((project, false)),
        Err(error) => error,
    };

    let mut project: Value = serde_json::from_slice(json)?;
    let mut recovered = false;
    if let Some(texture_sets) = project.get_mut("texture_sets").and_then(Value::as_array_mut) {
        for texture_set in texture_sets {
            if let Some(bake) = texture_set.get_mut("bake")
                && BakeSettings::deserialize(&*bake).is_err()
            {
                let mut settings = serde_json::to_value(BakeSettings::default())?;
                recover_bake_fields(&mut settings, bake, "");
                *bake = settings;
                recovered = true;
            }

            if let Some(baked) = texture_set.get_mut("baked") {
                if let Some(maps) = baked.as_array_mut() {
                    let count = maps.len();
                    maps.retain(|map| BakedMap::deserialize(map).is_ok());
                    recovered |= maps.len() != count;
                } else {
                    *baked = Value::Array(Vec::new());
                    recovered = true;
                }
            }
        }
    }

    if !recovered {
        return Err(error);
    }

    Ok((serde_json::from_value(project)?, true))
}

// Walk the default schema so unknown fields and input nesting cannot expand recovery
fn recover_bake_fields(recovered: &mut Value, original: &Value, path: &str) {
    let Some(fields) = recovered.pointer(path).and_then(Value::as_object) else {
        return;
    };

    let keys: Vec<_> = fields.keys().cloned().collect();
    for key in keys {
        let Some(value) = original.get(&key) else { continue };
        let field_path = format!("{path}/{key}");
        let Some(field) = recovered.pointer_mut(&field_path) else { continue };
        let previous = std::mem::replace(field, value.clone());
        if BakeSettings::deserialize(&*recovered).is_err() {
            if let Some(field) = recovered.pointer_mut(&field_path) {
                *field = previous;
            }

            // A bad nested field must not discard its readable siblings
            recover_bake_fields(recovered, value, &field_path);
        }
    }
}

pub(super) fn baked_map(name: &str) -> Option<BakedMap> {
    match name {
        "normal_base" => Some(BakedMap::Normal),
        "world_space_normals" => Some(BakedMap::WorldNormal),
        "ambient_occlusion" => Some(BakedMap::AmbientOcclusion),
        "bent_normals" => Some(BakedMap::BentNormal),
        _ => BakedMap::parse(name),
    }
}
