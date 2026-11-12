# Localization

TextureCraft's interface text goes through one lookup, `tr("English text")` in
`crates/ui-egui/src/i18n.rs`: the English string is the key, so English needs no table and a
missing translation falls back to English. Translations are embedded from the optional shared
craft-i18n catalogs; see [integration and contribution instructions](internationalization.md).
Text with values uses `trf("Saved {path}", &[("path", &p)])`; static labels translated where
they are shown are marked with `n_("…")`.

Pick the language in **Edit ▸ Settings ▸ Language** (it switches live). The first run follows
the operating system's language (the browser's in the web build).

## Status

The table records coverage before the initial catalog migration. Actual coverage depends on
the craft-i18n catalogs embedded in a build. Without catalogs, only English is offered.

UI strings: every `tr`/`trf`/`n_` literal in `crates/ui-egui/src` plus the menu catalog, panel
names, shortcut editor rows and the engine's channel, blend mode, mesh map, tool, procedural,
generator, filter and brush alpha names (**KEYS** strings, counted by the
`translation_coverage_report` test, which reports missing and unused keys without failing).

| Language | Code | UI strings | Dialogs, tooltips, help | Script support | Native review | Status | To `full` |
|---|---|---|---|---|---|---|---|
| English | `en` | KEYS (100%) | yes | Latin | yes | full | 0 h |
| Simplified Chinese | `zh-Hans` | KEYS (100%) | yes | CJK glyphs: craft-fonts (Noto Sans CJK SC) or system fonts on desktop; see below | no | full | 2 h (review) |
| Spanish | `es` | KEYS (100%) | yes | Latin | no | full | 1 h (review) |
| Hindi | `hi` | 0 | no | Devanagari shaping: not yet (egui has no complex shaping) | no | none | 12 h |
| Arabic | `ar` | KEYS (100%) | yes | Arabic glyphs: craft-fonts (Noto Sans Arabic) or system fonts on desktop; egui shapes the text and orders it right to left (panels keep the left-to-right layout) | yes | full | 0 h |
| French | `fr` | KEYS (100%) | yes | Latin | no | full | 1 h (review) |
| Portuguese (Brazil) | `pt-BR` | KEYS (100%) | yes | Latin | no | full | 1 h (review) |
| Indonesian | `id` | 0 | no | Latin | no | none | 3 h |
| Japanese | `ja` | KEYS (100%) | yes | CJK glyphs: craft-fonts (BIZ UDPGothic) or system fonts on desktop | no | full | 2 h (review) |
| German | `de` | KEYS (100%) | yes | Latin | no | full | 1 h (review) |
| Korean | `ko` | KEYS (100%) | yes | Hangul glyphs: system fonts on desktop (craft-fonts has no Korean face yet) | no | full | 2 h (review) |
| Vietnamese | `vi` | 0 | no | Latin with diacritics | no | none | 3 h |
| Italian | `it` | KEYS (100%) | yes | Latin | no | full | 1 h (review) |

Not translated on purpose: asset library names (materials, smart masks, alphas, environments),
user content (layer, texture set, project names), file format and technical names (sRGB8, L16F,
RGB, OpenGL, DirectX), and error details that come from the engine (shown after a translated
"Couldn't …:" prefix).

## Fonts for Chinese, Japanese, Korean and Arabic

egui's built-in fonts have no CJK or Arabic glyphs. `crates/ui-egui/src/fonts.rs` adds fallbacks after the
app's own fonts (Latin text and row heights keep the default font), loading them only when the
UI language needs them or the language picker shows every language's own name:

1. **craft-fonts** ([storytold/craft-fonts](https://github.com/storytold/craft-fonts)), embedded at
   build time when `CRAFT_FONTS_DIR` points at a checkout (`crates/ui-egui/build.rs`, the recipe in
   craft-fonts' `docs/integration.md`; `CRAFT_FONTS_REQUIRED=1` makes it mandatory for releases).
   Desktop builds embed BIZ UDPGothic Regular (Japanese), Noto Sans CJK SC Regular (Simplified
   Chinese) and Noto Sans Arabic Regular; the web build embeds BIZ UDPGothic (about 4.5 MB of the
   download) and Noto Sans Arabic (0.8 MB).
2. **System fonts** on desktop: Yu Gothic / Meiryo, Microsoft YaHei / SimSun and Malgun Gothic on
   Windows; Hiragino, PingFang / STHeiti and Apple SD Gothic Neo on macOS; Noto Sans CJK, WenQuanYi,
   Droid Sans Fallback and Nanum Gothic on Linux and FreeBSD. Arabic: Segoe UI / Tahoma / Arial on
   Windows, SF Arabic / Geeza Pro on macOS, Noto Sans Arabic or DejaVu Sans on Linux and FreeBSD.

**Web limitation:** a browser page can't read system fonts, so the web build shows CJK text only
with the fonts it embeds: Japanese (and most shared Han characters) and Arabic when built with
craft-fonts, none without it. A language no available font can draw appears in the picker under its English
name and can't be selected.
