//! Optional shared translation catalogs, embedded at build time.
use std::fmt::Write as _;
use std::path::Path;

pub fn generate(dir: Option<&Path>, required: bool) -> Result<String, String> {
    let mut source = String::from("pub static CRAFT_I18N: &craft_i18n::Embedded = &[\n");
    let Some(dir) = dir else {
        if required {
            return Err("CRAFT_I18N_REQUIRED=1 requires CRAFT_I18N_DIR".into());
        }
        source.push_str("];\n");
        return Ok(source);
    };
    if !dir.is_absolute() {
        return Err("CRAFT_I18N_DIR must be an absolute path".into());
    }
    let locales = dir.join("locales");
    let entries = std::fs::read_dir(&locales).map_err(|e| format!("{}: {e}", locales.display()))?;
    println!("cargo::rerun-if-changed={}", locales.display());
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    let mut count = 0;
    for path in paths {
        // Watch directories too: adding/removing an app catalog must rebuild the embedding.
        println!("cargo::rerun-if-changed={}", path.display());
        let app = path.join("texturecraft.tsv");
        if !app.is_file() {
            continue;
        }
        let locale = path.file_name().and_then(|n| n.to_str()).ok_or("non-UTF-8 locale")?;
        let common = path.join("common.tsv");
        let embed = |file: &Path| -> Result<String, String> {
            std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let name = file.to_str().ok_or("non-UTF-8 catalog path")?;
            println!("cargo::rerun-if-changed={name}");
            Ok(format!("include_str!({name:?})"))
        };
        let shared = if common.is_file() { embed(&common)? } else { "\"\"".into() };
        let app = embed(&app)?;
        let _ = writeln!(source, "    ({locale:?}, {shared}, {app}),");
        count += 1;
    }
    if required && count == 0 {
        return Err("no TextureCraft translation catalogs found".into());
    }
    source.push_str("];\n");
    Ok(source)
}

pub fn embed() {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    let dir = std::env::var_os("CRAFT_I18N_DIR").map(std::path::PathBuf::from);
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v == "1");
    let source = match generate(dir.as_deref(), required) {
        Ok(source) => source,
        Err(error) => {
            if required {
                println!("cargo::error={error}");
            } else {
                println!("cargo::warning=building without craft-i18n: {error}");
            }
            "pub static CRAFT_I18N: &craft_i18n::Embedded = &[];\n".into()
        }
    };
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_i18n.rs");
    if let Err(error) = std::fs::write(&out, source) {
        println!("cargo::error=writing {}: {error}", out.display());
    }
}
