//! UI text lookup. Keys are the English strings, so English needs no table and a missing
//! translation falls back to English. Every user-visible literal goes through [`tr`] (or [`trf`]
//! for text with `{name}` placeholders); each language is a static `(English, translation)`
//! table, indexed into a hash map the first time the language is used. `Prefs::language` picks it.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

mod ar;
mod de;
mod es;
mod fr;
mod it;
mod ja;
mod ko;
mod pt_br;
mod zh_hans;

pub struct Language {
    /// BCP 47 code ("en", "pt-BR", "zh-Hans").
    pub code: &'static str,
    /// The language's own name, for the picker.
    pub name: &'static str,
    /// Its English name (the picker's fallback when no font can draw `name`).
    pub english: &'static str,
    /// `(English, translation)` pairs, in any order.
    pub table: &'static [(&'static str, &'static str)],
}

const COUNT: usize = 10;

pub static LANGUAGES: [Language; COUNT] = [
    Language { code: "en", name: "English", english: "English", table: &[] },
    Language { code: "fr", name: "Français", english: "French", table: fr::TABLE },
    Language { code: "de", name: "Deutsch", english: "German", table: de::TABLE },
    Language { code: "es", name: "Español", english: "Spanish", table: es::TABLE },
    Language { code: "pt-BR", name: "Português (Brasil)", english: "Portuguese (Brazil)", table: pt_br::TABLE },
    Language { code: "it", name: "Italiano", english: "Italian", table: it::TABLE },
    Language { code: "ja", name: "日本語", english: "Japanese", table: ja::TABLE },
    Language { code: "zh-Hans", name: "简体中文", english: "Chinese (Simplified)", table: zh_hans::TABLE },
    Language { code: "ko", name: "한국어", english: "Korean", table: ko::TABLE },
    Language { code: "ar", name: "العربية", english: "Arabic", table: ar::TABLE },
];

static MAPS: [OnceLock<HashMap<&'static str, &'static str>>; COUNT] = [const { OnceLock::new() }; COUNT];
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Display labels belong to the UI; the model exposes only map identities
pub(crate) fn baked_map_label(map: texturecraft_engine::core::BakedMap) -> &'static str {
    use texturecraft_engine::core::BakedMap;

    match map {
        BakedMap::Normal => n_("Normal"),
        BakedMap::WorldNormal => n_("World Space Normal"),
        BakedMap::Id => n_("ID"),
        BakedMap::AmbientOcclusion => n_("Ambient Occlusion"),
        BakedMap::Curvature => n_("Curvature"),
        BakedMap::Position => n_("Position"),
        BakedMap::Thickness => n_("Thickness"),
        BakedMap::Height => n_("Height"),
        BakedMap::BentNormal => n_("Bent Normals"),
        BakedMap::Opacity => n_("Opacity"),
    }
}

/// Switch the UI language (unknown codes fall back to English).
pub fn set_language(code: &str) {
    CURRENT.store(LANGUAGES.iter().position(|l| l.code == code).unwrap_or(0), Ordering::Relaxed);
}

/// The current UI language.
pub fn current() -> &'static Language {
    LANGUAGES.get(CURRENT.load(Ordering::Relaxed)).unwrap_or(&LANGUAGES[0])
}

/// `text` in the current language (unchanged when it has no translation).
pub fn tr(text: &str) -> &str {
    let i = CURRENT.load(Ordering::Relaxed);
    let (Some(lang), Some(map)) = (LANGUAGES.get(i), MAPS.get(i)) else { return text };
    if lang.table.is_empty() {
        return text;
    }
    map.get_or_init(|| lang.table.iter().copied().collect()).get(text).copied().unwrap_or(text)
}

/// Marks a static string for translation where it is defined (gettext's `N_`); it is translated
/// with [`tr`] where it is shown. The key test collects both.
pub const fn n_(text: &'static str) -> &'static str {
    text
}

/// `template` translated, with each `{name}` replaced by its value in `args`.
pub fn trf(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = tr(template);
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').and_then(|close| args.iter().find(|(k, _)| *k == &after[..close]).map(|(_, v)| (close, v))) {
            Some((close, v)) => {
                use std::fmt::Write as _;
                let _ = write!(out, "{v}");
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The supported language for a locale such as "de-AT", "pt_BR.UTF-8" or "zh-Hant-TW" ("en" when
/// none matches). Chinese locales get Simplified Chinese; Portuguese ones get Brazilian Portuguese.
pub fn language_for_locale(locale: &str) -> &'static str {
    let lang = locale.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    match lang.as_str() {
        "pt" => "pt-BR",
        "zh" => "zh-Hans",
        l => LANGUAGES.iter().find(|x| x.code == l).map_or("en", |x| x.code),
    }
}

/// The operating system's (or browser's) UI language, for the first run.
pub fn os_language() -> &'static str {
    #[cfg(not(target_arch = "wasm32"))]
    let locale = sys_locale::get_locale();
    #[cfg(target_arch = "wasm32")]
    let locale = web_sys::window().and_then(|w| w.navigator().language());
    locale.map_or("en", |l| language_for_locale(&l))
}

/// Tests that switch the global UI language, or draw the UI under it, run one at a time.
#[cfg(test)]
pub(crate) static TEST_LANGUAGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::TEST_LANGUAGE as LANGUAGE;
    use super::*;

    /// Every key the UI looks up: `tr("…")` / `trf("…"` literals in the sources, plus the
    /// static catalogs whose labels are translated where they are shown.
    fn keys() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut keys = Vec::new();
        let mut directories = vec![dir];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() && path.file_name().is_some_and(|name| name != "i18n") {
                    directories.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    literals(&std::fs::read_to_string(path).unwrap(), &mut keys);
                }
            }
        }
        keys.extend(catalog_keys().into_iter().map(str::to_owned));
        keys.sort();
        keys.dedup();
        keys
    }

    /// The labels of the menu catalog, panels, shortcuts, shelf tabs and engine enums that are
    /// translated where they are shown.
    fn catalog_keys() -> Vec<&'static str> {
        use texturecraft_engine::core::{BlendMode, Channel, EffectKind, GeneratorKind, ProceduralKind};
        use texturecraft_engine::filter_lib::{FILTERS, build, scalar_params};
        let mut k: Vec<&'static str> = Vec::new();
        k.extend(crate::menus::TOP);
        for i in crate::menus::CATALOG.iter().filter(|i| i.label != "-") {
            k.push(i.label);
            k.extend(i.sub);
        }
        k.extend(crate::docking::VIEWS.iter().map(|(_, p)| p.name()));
        for b in crate::keymap::bindings() {
            k.push(b.label);
            k.push(b.group);
        }
        k.extend(crate::shelf::TABS.iter().map(|t| t.2));
        k.extend(Channel::ALL.iter().flat_map(|c| [c.label(), c.short()]));
        k.extend(BlendMode::all().map(|m| m.label()));
        k.extend(texturecraft_engine::formats::export::VirtualMap::ALL.iter().map(|m| m.label()));
        k.extend(texturecraft_engine::Tool::ALL.iter().map(|t| t.label()));
        k.extend(ProceduralKind::ALL.iter().map(|p| p.label()));
        k.extend(GeneratorKind::ALL.iter().map(|g| g.label()));
        k.extend(texturecraft_engine::paint::BrushAlpha::ALL.iter().map(|a| a.label()));
        k.extend(texturecraft_engine::geom::primitives::BUILTINS.iter().map(|(_, l)| *l));
        for f in FILTERS {
            k.extend([f.label, f.group]);
            if let Ok(EffectKind::Filter(x)) = build(f.id, &serde_json::Value::Null) {
                k.push(x.label());
                k.extend(scalar_params(&x).into_iter().map(|p| p.1));
            }
        }
        // Effect, view and camera names the engine gives.
        k.extend([
            "Paint",
            "Path",
            "Fill",
            "Generator",
            "Levels",
            "Color selection",
            "Compare mask",
            "Anchor point",
            "Layer mask",
            texturecraft_engine::camera_cmds::DEFAULT_CAMERA,
            crate::dock::SOLO_HINT,
        ]);
        k.retain(|s| !s.is_empty() && !UNTRANSLATED.contains(s));
        k
    }

    /// String literals passed straight to `tr(` or `trf(` in `src`.
    fn literals(src: &str, out: &mut Vec<String>) {
        let src = src
            .split(
                "#[cfg(test)]
mod tests",
            )
            .next()
            .unwrap_or(src);
        for pat in ["tr(", "trf(", "n_("] {
            let mut rest = src;
            while let Some(i) = rest.find(pat) {
                let before = rest[..i].chars().next_back();
                rest = rest[i + pat.len()..].trim_start();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let Some(after) = rest.strip_prefix('"') else { continue };
                rest = after;
                let mut s = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some(c) => s.push(c),
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
        }
    }

    fn placeholders(s: &str) -> Vec<&str> {
        let mut v: Vec<&str> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(name, _)| name)).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn every_key_is_translated_in_every_language() {
        let keys = keys();
        assert!(keys.len() > 300, "found only {} keys", keys.len());
        let mut missing = Vec::new();
        for l in LANGUAGES.iter().skip(1) {
            let map: HashMap<&str, &str> = l.table.iter().copied().collect();
            assert_eq!(map.len(), l.table.len(), "{}: duplicate keys", l.code);
            for k in &keys {
                if !map.contains_key(k.as_str()) {
                    missing.push(format!("{}: {k:?}", l.code));
                }
            }
        }
        assert!(missing.is_empty(), "{} missing translations:\n{}", missing.len(), missing.join("\n"));
    }

    /// Catalog labels that read the same in every language (names and formats).
    const UNTRANSLATED: &[&str] = &["RGB", "RGBA"];

    #[test]
    fn translations_keep_placeholders_and_are_not_empty() {
        for l in &LANGUAGES {
            for (en, t) in l.table {
                assert!(!t.trim().is_empty(), "{}: empty translation of {en:?}", l.code);
                assert_eq!(placeholders(en), placeholders(t), "{}: placeholders of {en:?} in {t:?}", l.code);
                assert_eq!(en.ends_with("..."), t.ends_with("...") || t.ends_with('…'), "{}: ellipsis of {en:?} in {t:?}", l.code);
            }
        }
    }

    #[test]
    fn lookup_formatting_and_fallbacks() {
        let _lock = LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_language("xx");
        assert_eq!(current().code, "en");
        assert_eq!(tr("Layers"), "Layers");
        assert_eq!(trf("Saved {path}", &[("path", &"a.tcraft")]), "Saved a.tcraft");
        assert_eq!(trf("{a} of {b} {c}", &[("a", &1), ("b", &2)]), "1 of 2 {c}");
        assert_eq!(trf("{unclosed", &[]), "{unclosed");
        for (locale, code) in
            [("de-AT", "de"), ("pt_PT.UTF-8", "pt-BR"), ("zh-Hant-TW", "zh-Hans"), ("ja", "ja"), ("ko_KR", "ko"), ("ar-EG", "ar"), ("nl-NL", "en"), ("", "en")]
        {
            assert_eq!(language_for_locale(locale), code, "{locale}");
        }
        set_language("de");
        assert_eq!(current().code, "de");
        assert_eq!(tr("not a key"), "not a key");
        set_language("en");
    }

    #[test]
    fn lookup_is_fast() {
        let _lock = LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys = keys();
        set_language("ja");
        let t = std::time::Instant::now();
        let mut n = 0usize;
        for _ in 0..100 {
            for k in &keys {
                n += tr(k).len();
            }
        }
        let per = t.elapsed().as_secs_f64() * 1e9 / (100 * keys.len()) as f64;
        set_language("en");
        assert!(n > 0);
        // A frame looks up a few hundred strings: well under a microsecond each.
        assert!(per < 1000.0, "{per:.0} ns per lookup");
    }
}
