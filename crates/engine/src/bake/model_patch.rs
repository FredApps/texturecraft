use serde_json::{Value, json};
use texturecraft_core::BakeSettings;

use crate::Result;
use crate::commands::bad;

/// Only the changed fields, using the persisted model's names and enum values
pub fn settings_patch(before: &BakeSettings, after: &BakeSettings) -> Value {
    changed_fields(&json!(before), &json!(after)).unwrap_or_else(|| json!({}))
}

fn changed_fields(before: &Value, after: &Value) -> Option<Value> {
    if before == after {
        return None;
    }

    if let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
        let fields = after
            .iter()
            .filter_map(|(name, value)| changed_fields(before.get(name).unwrap_or(&Value::Null), value).map(|change| (name.clone(), change)))
            .collect();
        Some(Value::Object(fields))
    } else {
        Some(after.clone())
    }
}

/// Merge known fields only; null is a value, so optional settings can be cleared
pub(super) fn apply_patch(model: &mut Value, patch: &Value, command: &str) -> Result<()> {
    let fields = patch.as_object().ok_or_else(|| bad(command, "bake settings patch must be an object"))?;
    for (name, value) in fields {
        let target = model.get_mut(name).ok_or_else(|| bad(command, format!("unknown bake setting `{name}`")))?;
        if target.is_object() {
            apply_patch(target, value, command)?;
        } else {
            *target = value.clone();
        }
    }
    Ok(())
}
