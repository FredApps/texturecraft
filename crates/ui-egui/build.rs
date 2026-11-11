//! Embeds the optional craft-fonts UI fonts (storytold/craft-fonts) when `CRAFT_FONTS_DIR` points
//! at a checkout: Inter for the interface (from its Latin catalog) and the CJK and Arabic
//! faces; without it `CRAFT_FONTS` is empty and the UI uses egui's and the system's fonts. The
//! recipe follows craft-fonts' `docs/integration.md`; no font file is committed here. The Latin
//! faces are embedded without their Private Use Area ([`without_private_use`]).

mod i18n_build;

use std::fmt::Write as _;
use std::path::PathBuf;

/// The UI faces TextureCraft embeds (family, style) per manifest: desktop, then web (the web build
/// has a download budget: of the CJK faces only the Japanese one, ~4.5 MB).
const MANIFESTS: &[(&str, Faces, Faces)] = &[
    ("fonts/manifest.txt", &[("BIZ UDPGothic", "Regular"), ("Noto Sans CJK SC", "Regular"), ARABIC], &[("BIZ UDPGothic", "Regular"), ARABIC]),
    // The interface font, regular and semibold (0.8 MB).
    (LATIN_MANIFEST, LATIN, LATIN),
];
/// The Arabic face (0.8 MB, so the web build has it too).
const ARABIC: (&str, &str) = ("Noto Sans Arabic", "Regular");
/// Newer than the core catalog: a checkout without it keeps the default interface font.
const LATIN_MANIFEST: &str = "fonts/latin-manifest.txt";
const LATIN: Faces = &[("Inter", "Regular"), ("Inter", "SemiBold")];
/// (family, style) pairs of a manifest.
type Faces = &'static [(&'static str, &'static str)];

fn main() {
    i18n_build::embed();
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").map(PathBuf::from) {
        match MANIFESTS.iter().map(|(rel, desktop, web)| craft_fonts(&dir, rel, desktop, web)).collect::<Result<String, String>>() {
            Ok(entries) => src.push_str(&entries),
            Err(e) if std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() => {
                println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display());
            }
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// One `CraftFont { .. }` initialiser per wanted line of manifest `rel`.
fn craft_fonts(dir: &std::path::Path, rel: &str, desktop: &[(&str, &str)], web: &[(&str, &str)]) -> Result<String, String> {
    let manifest = dir.join(rel);
    println!("cargo::rerun-if-changed={}", manifest.display());
    if rel == LATIN_MANIFEST && !manifest.exists() {
        return Ok(String::new());
    }
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let wanted = if wasm { web } else { desktop };
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if !wanted.contains(&(*family, *style)) {
            continue;
        }
        let mut path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        if rel == LATIN_MANIFEST {
            let bytes = std::fs::read(&path).map_err(|e| format!("{file}: {e}"))?;
            let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join(path.file_name().unwrap_or_default());
            std::fs::write(&out, without_private_use(&bytes).map_err(|e| format!("{file}: {e}"))?).map_err(|e| format!("{}: {e}", out.display()))?;
            path = out;
        }
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    Ok(out)
}

/// `font` without the characters of the Private Use Area (U+E000–U+F8FF): the icon font (Phosphor)
/// draws there, and Inter, ahead of it in the fallback list, has alternates of its own there that
/// would show instead of the icons. Everything else (layout features, hinting) is kept, as
/// craft-libs' `craft-fonts-build` subsets.
fn without_private_use(font: &[u8]) -> Result<Vec<u8>, String> {
    use skera::{Plan, SubsetFlags, subset_font};
    use skrifa::raw::collections::IntSet;
    use skrifa::raw::types::NameId;
    use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
    let font = FontRef::new(font).map_err(|e| e.to_string())?;
    let mut unicodes = IntSet::<u32>::empty();
    unicodes.extend(font.charmap().mappings().map(|(c, _)| c).filter(|c| !(0xE000..=0xF8FF).contains(c)));
    let mut name_ids = IntSet::<NameId>::empty();
    name_ids.insert_range(NameId::new(0)..=NameId::new(6));
    let mut languages = IntSet::<u16>::empty();
    languages.insert(0x0409);
    let plan = Plan::new(
        &IntSet::<GlyphId>::empty(),
        &unicodes,
        &font,
        SubsetFlags::default(),
        &IntSet::<Tag>::empty(),
        &IntSet::<Tag>::all(),
        &IntSet::<Tag>::all(),
        &name_ids,
        &languages,
    );
    subset_font(&font, &plan).map_err(|e| e.to_string())
}
