//! The filter library (the Filters shelf): adjustment, blur and distortion filters for
//! layer content and masks, and material finishes (content only). `effect.add.filter` adds one
//! by id; `filter.list` lists them with their default parameters.

use serde_json::{Value, json};
use texturecraft_core::{Channel, EffectKind, Fill, Filter, Levels, Procedural, ProceduralKind, Projection, Source};

use crate::commands::{CommandSpec, always, bad, color_of, color_param, f32p, u64p};
use crate::{Result, cmd};

/// A filter of the library.
#[derive(Clone, Copy, Debug)]
pub struct FilterDef {
    pub id: &'static str,
    pub label: &'static str,
    /// Shelf group.
    pub group: &'static str,
    /// Usable on masks too (material finishes only fill layer content).
    pub mask: bool,
}

impl FilterDef {
    /// Whether it builds a filter effect (levels and material finishes are effect types of their own)
    pub fn is_filter_effect(&self) -> bool {
        matches!(build(self.id, &Value::Null), Ok(EffectKind::Filter(_)))
    }
}

/// The library entries that build filter effects, in shelf order, worked out once
pub fn filter_effects() -> &'static [&'static FilterDef] {
    static FILTER_EFFECTS: std::sync::LazyLock<Vec<&'static FilterDef>> =
        std::sync::LazyLock::new(|| FILTERS.iter().filter(|def| def.is_filter_effect()).collect());
    &FILTER_EFFECTS
}

const fn def(id: &'static str, label: &'static str, group: &'static str, mask: bool) -> FilterDef {
    FilterDef { id, label, group, mask }
}

pub const FILTERS: &[FilterDef] = &[
    def("hsl", "HSL Perceptive", "Adjustments", true),
    def("levels", "Levels", "Adjustments", true),
    def("contrastLuminosity", "Contrast/Luminosity", "Adjustments", true),
    def("colorBalance", "Color Balance", "Adjustments", true),
    def("gradientMap", "Gradient Map", "Adjustments", true),
    def("posterize", "Posterize", "Adjustments", true),
    def("clamp", "Clamp", "Adjustments", true),
    def("histogramScan", "Histogram Scan", "Adjustments", true),
    def("histogramRange", "Histogram Range", "Adjustments", true),
    def("histogramShift", "Histogram Shift", "Adjustments", true),
    def("invert", "Invert", "Adjustments", true),
    def("threshold", "Threshold", "Adjustments", true),
    def("grayscale", "Grayscale", "Adjustments", false),
    def("edgeDetect", "Edge Detect", "Adjustments", true),
    def("normalInvert", "Normal Invert", "Adjustments", false),
    def("bakedLighting", "Baked Lighting", "Adjustments", false),
    def("replaceColor", "Replace Color", "Adjustments", false),
    def("blur", "Blur", "Blur and sharpen", true),
    def("blurDirectional", "Blur Directional", "Blur and sharpen", true),
    def("slopeBlur", "Slope Blur", "Blur and sharpen", true),
    def("sharpen", "Sharpen", "Blur and sharpen", true),
    def("highPass", "High Pass", "Blur and sharpen", true),
    def("bevel", "Bevel", "Blur and sharpen", true),
    def("glow", "Glow", "Blur and sharpen", true),
    def("warp", "Warp", "Distort", true),
    def("warpDirectional", "Warp Directional", "Distort", true),
    def("mosaic", "Mosaic", "Distort", true),
    def("matFinishGrind", "MatFinish Grind", "Material finish", false),
    def("matFinishRaw", "MatFinish Raw", "Material finish", false),
    def("matFinishSanded", "MatFinish Sanded", "Material finish", false),
    def("matFinishHammered", "MatFinish Hammered", "Material finish", false),
];

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().replace([' ', '_', '-', '/'], "")
}

/// The library filter `id` (case and separators ignored; labels work too).
pub fn find(id: &str) -> Option<&'static FilterDef> {
    let n = norm(id);
    FILTERS.iter().find(|f| norm(f.id) == n || norm(f.label) == n)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "filter.list",
        "List filters",
        [],
        None,
        "{} — the filter library: id, label, group, whether masks take it, default parameters",
        always,
        |_, _| Ok(Value::Array(
            FILTERS
                .iter()
                .map(|f| json!({"id": f.id, "label": f.label, "group": f.group, "mask": f.mask, "defaults": build(f.id, &Value::Null).ok()}))
                .collect()
        ))
    )]
}

/// The effect for library filter `id` with its defaults, then the parameters in `p`.
pub fn build(id: &str, p: &Value) -> Result<EffectKind> {
    let cmd = "effect.add.filter";
    let d = find(id).ok_or_else(|| bad(cmd, format!("unknown filter `{id}` (see filter.list)")))?;
    let mut f = match d.id {
        "invert" => Filter::Invert,
        "threshold" => Filter::Threshold { level: 0.5 },
        "blur" => Filter::Blur { radius: 2.0 },
        "sharpen" => Filter::Sharpen { amount: 1.0 },
        "hsl" => Filter::Hsl { hue: 0.0, saturation: 0.0, lightness: 0.0 },
        "contrastLuminosity" => Filter::ContrastLuminosity { contrast: 0.3, luminosity: 0.0 },
        "colorBalance" => Filter::ColorBalance { shadows: [0.0; 3], midtones: [0.0; 3], highlights: [0.0; 3] },
        "gradientMap" => Filter::GradientMap { colors: vec![[0.02, 0.02, 0.04], [0.45, 0.16, 0.05], [1.0, 0.85, 0.55]] },
        "posterize" => Filter::Posterize { levels: 4.0 },
        "clamp" => Filter::Clamp { min: 0.2, max: 0.8 },
        "histogramScan" => Filter::HistogramScan { position: 0.5, contrast: 0.5 },
        "histogramRange" => Filter::HistogramRange { range: 0.5, position: 0.5 },
        "histogramShift" => Filter::HistogramShift { offset: 0.25 },
        "warp" => Filter::Warp { intensity: 16.0, scale: 4.0, seed: 1 },
        "blurDirectional" => Filter::DirectionalBlur { radius: 16.0, angle: 0.0 },
        "slopeBlur" => Filter::SlopeBlur { radius: 16.0, scale: 4.0, seed: 1 },
        "highPass" => Filter::HighPass { radius: 8.0 },
        "edgeDetect" => Filter::EdgeDetect { width: 2.0 },
        "warpDirectional" => Filter::DirectionalWarp { intensity: 16.0, angle: 0.0, scale: 4.0, seed: 1 },
        "grayscale" => Filter::Grayscale,
        "normalInvert" => Filter::NormalInvert,
        "bevel" => Filter::Bevel { width: 16.0 },
        "mosaic" => Filter::Mosaic { size: 32.0 },
        "glow" => Filter::Glow { radius: 16.0, amount: 1.0 },
        "replaceColor" => Filter::ReplaceColor { from: [0.5, 0.05, 0.05], to: [0.05, 0.15, 0.5], tolerance: 0.2, softness: 0.15 },
        "bakedLighting" => Filter::BakedLighting { azimuth: 45.0, elevation: 45.0, light: 0.8, ambient: 0.4, occlusion: 1.0 },
        "levels" => {
            let mut l = Levels::default();
            crate::mask_cmds::levels_from(p, &mut l);
            return Ok(EffectKind::Levels(l));
        }
        finish => return mat_finish(finish).map(EffectKind::Material).ok_or_else(|| bad(cmd, format!("unknown filter `{id}`"))),
    };
    apply(p, &mut f, cmd)?;
    Ok(EffectKind::Filter(f))
}

/// A material finish: procedural roughness and height (tri-planar, so it is seamless).
fn mat_finish(id: &str) -> Option<Fill> {
    let (kind, scale, rough, height) = match id {
        "matFinishGrind" => (ProceduralKind::Brushed, 6.0, [0.22, 0.42], 0.012),
        "matFinishRaw" => (ProceduralKind::Clouds, 8.0, [0.45, 0.75], 0.02),
        "matFinishSanded" => (ProceduralKind::Noise, 3.0, [0.38, 0.58], 0.008),
        "matFinishHammered" => (ProceduralKind::Hammered, 6.0, [0.25, 0.45], 0.08),
        _ => return None,
    };
    let pattern = Procedural { scale, ..Procedural::new(kind) };
    let mut fill = Fill { projection: Projection::Triplanar { scale: 1.0, sharpness: 8.0, offset: [0.0; 3] }, ..Fill::default() };
    fill.sources.insert(Channel::Roughness, Source::Procedural { pattern: pattern.clone(), low: [rough[0]; 3], high: [rough[1]; 3] });
    fill.sources.insert(Channel::Height, Source::Procedural { pattern, low: [-height; 3], high: [height; 3] });
    Some(fill)
}

/// Scalar parameters of a filter for editors: (param key, label, value, min, max). Colors
/// (color balance, gradient map) and the HSL hue have their own editors.
pub fn scalar_params(f: &Filter) -> Vec<(&'static str, &'static str, f32, f32, f32)> {
    match f {
        Filter::Blur { radius } => vec![("radius", "Intensity (texels at 2K)", *radius, 0.0, 128.0)],
        Filter::Sharpen { amount } => vec![("amount", "Amount", *amount, 0.0, 5.0)],
        Filter::Threshold { level } => vec![("level", "Level", *level, 0.0, 1.0)],
        Filter::Hsl { saturation, lightness, .. } => {
            vec![("saturation", "Saturation", *saturation, -1.0, 1.0), ("lightness", "Lightness", *lightness, -1.0, 1.0)]
        }
        Filter::ContrastLuminosity { contrast, luminosity } => {
            vec![("contrast", "Contrast", *contrast, -1.0, 1.0), ("luminosity", "Luminosity", *luminosity, -1.0, 1.0)]
        }
        Filter::Posterize { levels } => vec![("levels", "Steps", *levels, 2.0, 64.0)],
        Filter::Clamp { min, max } => vec![("min", "Minimum", *min, 0.0, 1.0), ("max", "Maximum", *max, 0.0, 1.0)],
        Filter::HistogramScan { position, contrast } => vec![("position", "Position", *position, 0.0, 1.0), ("contrast", "Contrast", *contrast, 0.0, 1.0)],
        Filter::HistogramRange { range, position } => vec![("range", "Range", *range, 0.0, 1.0), ("position", "Position", *position, 0.0, 1.0)],
        Filter::HistogramShift { offset } => vec![("offset", "Offset", *offset, -1.0, 1.0)],
        Filter::Warp { intensity, scale, seed } => {
            vec![
                ("intensity", "Intensity (texels at 2K)", *intensity, 0.0, 128.0),
                ("scale", "Noise scale", *scale, 0.5, 64.0),
                ("seed", "Seed", *seed as f32, 0.0, 999.0),
            ]
        }
        Filter::DirectionalBlur { radius, angle } => {
            vec![("radius", "Intensity (texels at 2K)", *radius, 0.0, 128.0), ("angle", "Angle (degrees)", *angle, 0.0, 360.0)]
        }
        Filter::SlopeBlur { radius, scale, seed } => {
            vec![
                ("radius", "Intensity (texels at 2K)", *radius, 0.0, 128.0),
                ("scale", "Slope scale", *scale, 0.5, 64.0),
                ("seed", "Seed", *seed as f32, 0.0, 999.0),
            ]
        }
        Filter::HighPass { radius } => vec![("radius", "Intensity (texels at 2K)", *radius, 0.0, 128.0)],
        Filter::EdgeDetect { width } => vec![("width", "Width (texels at 2K)", *width, 0.0, 64.0)],
        Filter::DirectionalWarp { intensity, angle, scale, seed } => {
            vec![
                ("intensity", "Intensity (texels at 2K)", *intensity, 0.0, 128.0),
                ("angle", "Angle (degrees)", *angle, 0.0, 360.0),
                ("scale", "Noise scale", *scale, 0.5, 64.0),
                ("seed", "Seed", *seed as f32, 0.0, 999.0),
            ]
        }
        Filter::Bevel { width } => vec![("width", "Width (texels at 2K)", *width, 0.0, 128.0)],
        Filter::Mosaic { size } => vec![("size", "Cell size (texels at 2K)", *size, 1.0, 256.0)],
        Filter::Glow { radius, amount } => {
            vec![("radius", "Radius (texels at 2K)", *radius, 0.0, 128.0), ("amount", "Amount", *amount, 0.0, 5.0)]
        }
        Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion } => vec![
            ("azimuth", "Light azimuth (degrees)", *azimuth, 0.0, 360.0),
            ("elevation", "Light elevation (degrees)", *elevation, -90.0, 90.0),
            ("light", "Light intensity", *light, 0.0, 4.0),
            ("ambient", "Ambient", *ambient, 0.0, 2.0),
            ("occlusion", "Ambient occlusion", *occlusion, 0.0, 1.0),
        ],
        Filter::ReplaceColor { tolerance, softness, .. } => {
            vec![("tolerance", "Tolerance", *tolerance, 0.0, 1.0), ("softness", "Softness", *softness, 0.0, 1.0)]
        }
        Filter::Invert | Filter::ColorBalance { .. } | Filter::GradientMap { .. } | Filter::Grayscale | Filter::NormalInvert => Vec::new(),
    }
}

/// Set the parameters of `f` given in `p` (clamped to their ranges); `effect.set` uses it too.
pub(crate) fn apply(p: &Value, f: &mut Filter, cmd: &str) -> Result<()> {
    let set = |slot: &mut f32, key: &str, lo: f32, hi: f32| {
        if let Some(v) = f32p(p, key) {
            *slot = v.clamp(lo, hi);
        }
    };
    let seed = |slot: &mut u32| {
        if let Some(v) = u64p(p, "seed") {
            *slot = u32::try_from(v).unwrap_or(u32::MAX);
        }
    };
    let angle = |slot: &mut f32| {
        if let Some(v) = f32p(p, "angle") {
            *slot = v.rem_euclid(360.0);
        }
    };
    match f {
        Filter::Invert | Filter::Grayscale | Filter::NormalInvert => {}
        Filter::Threshold { level } => set(level, "level", 0.0, 1.0),
        Filter::Blur { radius } => set(radius, "radius", 0.0, 256.0),
        Filter::Sharpen { amount } => set(amount, "amount", 0.0, 10.0),
        Filter::Hsl { hue, saturation, lightness } => {
            set(hue, "hue", -1.0, 1.0);
            set(saturation, "saturation", -1.0, 1.0);
            set(lightness, "lightness", -1.0, 1.0);
        }
        Filter::ContrastLuminosity { contrast, luminosity } => {
            set(contrast, "contrast", -1.0, 1.0);
            set(luminosity, "luminosity", -1.0, 1.0);
        }
        Filter::ColorBalance { shadows, midtones, highlights } => {
            for (key, slot) in [("shadows", shadows), ("midtones", midtones), ("highlights", highlights)] {
                if let Some(val) = p.get(key).filter(|v| !v.is_null()) {
                    let v = color_of(val).ok_or_else(|| bad(cmd, format!("`{key}` must be [r, g, b] offsets in -1..1")))?;
                    *slot = v.map(|x| x.clamp(-1.0, 1.0));
                }
            }
        }
        Filter::GradientMap { colors } => {
            if let Some(list) = p.get("colors") {
                let parsed: Option<Vec<[f32; 3]>> = list.as_array().and_then(|a| a.iter().map(|c| color_of(c).map(|v| v.map(|x| x.clamp(0.0, 1.0)))).collect());
                match parsed {
                    Some(c) if (1..=16).contains(&c.len()) => *colors = c,
                    _ => return Err(bad(cmd, "`colors` must be 1 to 16 colors ([r, g, b] linear or \"#rrggbb\")")),
                }
            }
        }
        Filter::Posterize { levels } => set(levels, "levels", 2.0, 64.0),
        Filter::Clamp { min, max } => {
            set(min, "min", 0.0, 1.0);
            set(max, "max", 0.0, 1.0);
        }
        Filter::HistogramScan { position, contrast } => {
            set(position, "position", 0.0, 1.0);
            set(contrast, "contrast", 0.0, 1.0);
        }
        Filter::HistogramRange { range, position } => {
            set(range, "range", 0.0, 1.0);
            set(position, "position", 0.0, 1.0);
        }
        Filter::HistogramShift { offset } => set(offset, "offset", -1.0, 1.0),
        Filter::Warp { intensity, scale, seed: s } => {
            set(intensity, "intensity", 0.0, 256.0);
            set(scale, "scale", 0.01, 1000.0);
            seed(s);
        }
        Filter::DirectionalBlur { radius, angle: a } => {
            set(radius, "radius", 0.0, 256.0);
            angle(a);
        }
        Filter::SlopeBlur { radius, scale, seed: s } => {
            set(radius, "radius", 0.0, 256.0);
            set(scale, "scale", 0.01, 1000.0);
            seed(s);
        }
        Filter::HighPass { radius } => set(radius, "radius", 0.0, 256.0),
        Filter::EdgeDetect { width } | Filter::Bevel { width } => set(width, "width", 0.0, 256.0),
        Filter::Mosaic { size } => set(size, "size", 1.0, 512.0),
        Filter::Glow { radius, amount } => {
            set(radius, "radius", 0.0, 256.0);
            set(amount, "amount", 0.0, 10.0);
        }
        Filter::ReplaceColor { from, to, tolerance, softness } => {
            for (key, slot) in [("from", from), ("to", to)] {
                if let Some(v) = color_param(cmd, p, key)? {
                    *slot = v.map(|x| x.clamp(0.0, 1.0));
                }
            }
            set(tolerance, "tolerance", 0.0, 1.0);
            set(softness, "softness", 0.0, 1.0);
        }
        Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion } => {
            if let Some(v) = f32p(p, "azimuth") {
                *azimuth = v.rem_euclid(360.0);
            }
            set(elevation, "elevation", -90.0, 90.0);
            set(light, "light", 0.0, 10.0);
            set(ambient, "ambient", 0.0, 10.0);
            set(occlusion, "occlusion", 0.0, 1.0);
        }
        Filter::DirectionalWarp { intensity, angle: a, scale, seed: s } => {
            set(intensity, "intensity", 0.0, 256.0);
            angle(a);
            set(scale, "scale", 0.01, 1000.0);
            seed(s);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filter_builds_and_finds_by_id_or_label() {
        for f in FILTERS {
            let kind = build(f.id, &Value::Null).unwrap_or_else(|e| panic!("{}: {e}", f.id));
            // Material finishes are content only; other filters also run on masks, where some
            // (grayscale, normal invert) change nothing and are not offered.
            assert_eq!(kind.content_only().is_none(), f.mask || matches!(kind, EffectKind::Filter(_)), "{}", f.id);
            assert_eq!(find(f.label).map(|d| d.id), Some(f.id));
        }
        assert!(build("nope", &Value::Null).is_err());
        let EffectKind::Filter(Filter::Posterize { levels }) = build("posterize", &json!({"levels": 1000})).unwrap() else { panic!() };
        assert_eq!(levels, 64.0);
        assert!(build("gradientMap", &json!({"colors": []})).is_err());
        assert!(build("colorBalance", &json!({"shadows": "x"})).is_err());
        let EffectKind::Filter(Filter::GradientMap { colors }) = build("gradientMap", &json!({"colors": [[0, 0, 0], "#ffffff"]})).unwrap() else { panic!() };
        assert_eq!(colors.len(), 2);
    }
}
