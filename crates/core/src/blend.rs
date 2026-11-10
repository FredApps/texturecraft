//! Per-channel blend modes and their CPU reference math (straight values, linear space).
//!
//! `a` is the value below (the stack so far), `b` the layer's value. The compositor mixes the
//! result back over `a` by `coverage × opacity × mask`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Passthrough,
    Disable,
    Replace,
    Multiply,
    Divide,
    InverseDivide,
    Darken,
    Lighten,
    LinearDodge,
    Subtract,
    InverseSubtract,
    Difference,
    Exclusion,
    SignedAddition,
    Overlay,
    Screen,
    LinearBurn,
    ColorBurn,
    ColorDodge,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    Tint,
    Saturation,
    Color,
    Value,
    NormalMapCombine,
    NormalMapDetail,
    NormalMapInverseDetail,
}

impl BlendMode {
    /// Every mode in menu order; `None` entries are the separators between menu groups.
    pub const MENU: [Option<BlendMode>; 39] = [
        Some(BlendMode::Normal),
        Some(BlendMode::Passthrough),
        Some(BlendMode::Disable),
        Some(BlendMode::Replace),
        None,
        Some(BlendMode::Multiply),
        Some(BlendMode::Divide),
        Some(BlendMode::InverseDivide),
        Some(BlendMode::Darken),
        Some(BlendMode::Lighten),
        None,
        Some(BlendMode::LinearDodge),
        Some(BlendMode::Subtract),
        Some(BlendMode::InverseSubtract),
        Some(BlendMode::Difference),
        Some(BlendMode::Exclusion),
        Some(BlendMode::SignedAddition),
        None,
        Some(BlendMode::Overlay),
        Some(BlendMode::Screen),
        Some(BlendMode::LinearBurn),
        Some(BlendMode::ColorBurn),
        Some(BlendMode::ColorDodge),
        None,
        Some(BlendMode::SoftLight),
        Some(BlendMode::HardLight),
        Some(BlendMode::VividLight),
        Some(BlendMode::LinearLight),
        Some(BlendMode::PinLight),
        None,
        Some(BlendMode::Tint),
        Some(BlendMode::Saturation),
        Some(BlendMode::Color),
        Some(BlendMode::Value),
        None,
        Some(BlendMode::NormalMapCombine),
        Some(BlendMode::NormalMapDetail),
        Some(BlendMode::NormalMapInverseDetail),
        None,
    ];

    pub fn all() -> impl Iterator<Item = BlendMode> {
        BlendMode::MENU.into_iter().flatten()
    }

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Passthrough => "Passthrough",
            BlendMode::Disable => "Disable",
            BlendMode::Replace => "Replace",
            BlendMode::Multiply => "Multiply",
            BlendMode::Divide => "Divide",
            BlendMode::InverseDivide => "Inverse divide",
            BlendMode::Darken => "Darken (Min)",
            BlendMode::Lighten => "Lighten (Max)",
            BlendMode::LinearDodge => "Linear dodge (Add)",
            BlendMode::Subtract => "Subtract",
            BlendMode::InverseSubtract => "Inverse Subtract",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::SignedAddition => "Signed addition (AddSub)",
            BlendMode::Overlay => "Overlay",
            BlendMode::Screen => "Screen",
            BlendMode::LinearBurn => "Linear burn",
            BlendMode::ColorBurn => "Color burn",
            BlendMode::ColorDodge => "Color dodge",
            BlendMode::SoftLight => "Soft light",
            BlendMode::HardLight => "Hard light",
            BlendMode::VividLight => "Vivid light",
            BlendMode::LinearLight => "Linear light",
            BlendMode::PinLight => "Pin light",
            BlendMode::Tint => "Tint",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Value => "Value",
            BlendMode::NormalMapCombine => "Normal map combine",
            BlendMode::NormalMapDetail => "Normal map detail",
            BlendMode::NormalMapInverseDetail => "Normal map inverse detail",
        }
    }

    /// Four-letter abbreviation shown in the compact layer row ("Norm", "Mult", …).
    pub fn short(self) -> &'static str {
        match self {
            BlendMode::Normal => "Norm",
            BlendMode::Passthrough => "Pass",
            BlendMode::Disable => "Dis",
            BlendMode::Replace => "Repl",
            BlendMode::Multiply => "Mult",
            BlendMode::Divide => "Div",
            BlendMode::InverseDivide => "IDiv",
            BlendMode::Darken => "Dark",
            BlendMode::Lighten => "Ligh",
            BlendMode::LinearDodge => "Add",
            BlendMode::Subtract => "Sub",
            BlendMode::InverseSubtract => "ISub",
            BlendMode::Difference => "Diff",
            BlendMode::Exclusion => "Excl",
            BlendMode::SignedAddition => "AdSb",
            BlendMode::Overlay => "Over",
            BlendMode::Screen => "Scrn",
            BlendMode::LinearBurn => "LBrn",
            BlendMode::ColorBurn => "CBrn",
            BlendMode::ColorDodge => "CDdg",
            BlendMode::SoftLight => "SftL",
            BlendMode::HardLight => "HrdL",
            BlendMode::VividLight => "VivL",
            BlendMode::LinearLight => "LinL",
            BlendMode::PinLight => "PinL",
            BlendMode::Tint => "Tint",
            BlendMode::Saturation => "Sat",
            BlendMode::Color => "Colr",
            BlendMode::Value => "Val",
            BlendMode::NormalMapCombine => "NmCb",
            BlendMode::NormalMapDetail => "NmDt",
            BlendMode::NormalMapInverseDetail => "NmID",
        }
    }

    /// Parse a label, short label or variant name (case-insensitive; punctuation ignored).
    pub fn parse(s: &str) -> Option<BlendMode> {
        let norm = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-', '(', ')'], "");
        let want = norm(s);
        BlendMode::all().find(|m| norm(m.label()) == want || norm(m.short()) == want || norm(&format!("{m:?}")) == want)
    }

    /// Modes that need all three components at once.
    pub fn is_non_separable(self) -> bool {
        matches!(
            self,
            BlendMode::Tint
                | BlendMode::Saturation
                | BlendMode::Color
                | BlendMode::Value
                | BlendMode::NormalMapCombine
                | BlendMode::NormalMapDetail
                | BlendMode::NormalMapInverseDetail
        )
    }
}

fn color_burn(a: f32, b: f32) -> f32 {
    if a >= 1.0 {
        1.0
    } else if b <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - a) / b).min(1.0)
    }
}

fn color_dodge(a: f32, b: f32) -> f32 {
    if a <= 0.0 {
        0.0
    } else if b >= 1.0 {
        1.0
    } else {
        (a / (1.0 - b)).min(1.0)
    }
}

fn soft_light(a: f32, b: f32) -> f32 {
    let (a, b) = (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
    if b <= 0.5 {
        return a - (1.0 - 2.0 * b) * a * (1.0 - a);
    }
    let d = if a <= 0.25 { ((16.0 * a - 12.0) * a + 4.0) * a } else { a.sqrt() };
    a + (2.0 * b - 1.0) * (d - a)
}

fn overlay(a: f32, b: f32) -> f32 {
    if a < 0.5 { 2.0 * a * b } else { 1.0 - 2.0 * (1.0 - a) * (1.0 - b) }
}

/// One component of a separable blend. Non-separable modes fall back to `Normal`.
pub fn blend1(mode: BlendMode, a: f32, b: f32) -> f32 {
    match mode {
        BlendMode::Normal | BlendMode::Passthrough | BlendMode::Replace => b,
        BlendMode::Disable => a,
        BlendMode::Multiply => a * b,
        BlendMode::Divide => a / b.max(1e-6),
        BlendMode::InverseDivide => b / a.max(1e-6),
        BlendMode::Darken => a.min(b),
        BlendMode::Lighten => a.max(b),
        BlendMode::LinearDodge => a + b,
        BlendMode::Subtract => a - b,
        BlendMode::InverseSubtract => b - a,
        BlendMode::Difference => (a - b).abs(),
        BlendMode::Exclusion => a + b - 2.0 * a * b,
        BlendMode::SignedAddition | BlendMode::LinearLight => a + 2.0 * b - 1.0,
        BlendMode::Overlay => overlay(a, b),
        BlendMode::Screen => 1.0 - (1.0 - a) * (1.0 - b),
        BlendMode::LinearBurn => a + b - 1.0,
        BlendMode::ColorBurn => color_burn(a, b),
        BlendMode::ColorDodge => color_dodge(a, b),
        BlendMode::SoftLight => soft_light(a, b),
        BlendMode::HardLight => overlay(b, a),
        BlendMode::VividLight => {
            if b < 0.5 {
                color_burn(a, 2.0 * b)
            } else {
                color_dodge(a, 2.0 * (b - 0.5))
            }
        }
        BlendMode::PinLight => {
            if b < 0.5 {
                a.min(2.0 * b)
            } else {
                a.max(2.0 * b - 1.0)
            }
        }
        BlendMode::Tint
        | BlendMode::Saturation
        | BlendMode::Color
        | BlendMode::Value
        | BlendMode::NormalMapCombine
        | BlendMode::NormalMapDetail
        | BlendMode::NormalMapInverseDetail => b,
    }
}

/// Luminance of a color (the W3C non-separable blend weights).
pub fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 && l - n > 1e-6 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 && x - l > 1e-6 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    if mx - mn <= 1e-6 {
        return [0.0; 3];
    }
    c.map(|v| (v - mn) * s / (mx - mn))
}

fn unpack(n: [f32; 3]) -> [f32; 3] {
    n.map(|v| v * 2.0 - 1.0)
}

fn pack(n: [f32; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    n.map(|v| v / len * 0.5 + 0.5)
}

/// Reoriented normal mapping (Barré-Brisebois & Hill): `detail` re-oriented onto `base` (both
/// packed into 0..1, as is the result).
pub fn rnm(base: [f32; 3], detail: [f32; 3]) -> [f32; 3] {
    let t = unpack(base);
    let t = [t[0], t[1], t[2] + 1.0];
    let u = unpack(detail);
    let u = [-u[0], -u[1], u[2]];
    let d = t[0] * u[0] + t[1] * u[1] + t[2] * u[2];
    pack([t[0] * d - u[0] * t[2], t[1] * d - u[1] * t[2], t[2] * d - u[2] * t[2]])
}

/// A three-component blend, including the non-separable modes.
pub fn blend3(mode: BlendMode, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Color => set_lum(b, lum(a)),
        BlendMode::Saturation => set_lum(set_sat(a, sat(b)), lum(a)),
        BlendMode::Value => {
            let va = a[0].max(a[1]).max(a[2]);
            let vb = b[0].max(b[1]).max(b[2]);
            if va <= 1e-6 { [vb; 3] } else { a.map(|v| v * vb / va) }
        }
        BlendMode::Tint => {
            let l = lum(a);
            b.map(|v| v * l)
        }
        BlendMode::NormalMapCombine => {
            let (na, nb) = (unpack(a), unpack(b));
            pack([na[0] + nb[0], na[1] + nb[1], na[2] * nb[2]])
        }
        BlendMode::NormalMapDetail => rnm(a, b),
        BlendMode::NormalMapInverseDetail => rnm(b, a),
        _ => [blend1(mode, a[0], b[0]), blend1(mode, a[1], b[1]), blend1(mode, a[2], b[2])],
    }
}

/// Mix `blended` over `a` by weight `w` (coverage × opacity × mask).
pub fn mix3(a: [f32; 3], blended: [f32; 3], w: f32) -> [f32; 3] {
    // A full-weight blend is exactly the blended value (`a + (b - a)` can be an ulp off it): what
    // lies under it does not leak in, so skipping the layers it hides changes nothing at all.
    if w >= 1.0 {
        return blended;
    }
    [a[0] + (blended[0] - a[0]) * w, a[1] + (blended[1] - a[1]) * w, a[2] + (blended[2] - a[2]) * w]
}

#[cfg(test)]
mod tests {
    use super::*;

    const AS: [f32; 4] = [0.0, 0.25, 0.5, 1.0];
    const BS: [f32; 4] = [0.0, 0.3, 0.7, 1.0];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn separable_modes_match_hand_computed_values() {
        for a in AS {
            for b in BS {
                assert!(close(blend1(BlendMode::Normal, a, b), b));
                assert!(close(blend1(BlendMode::Disable, a, b), a));
                assert!(close(blend1(BlendMode::Multiply, a, b), a * b));
                assert!(close(blend1(BlendMode::Screen, a, b), 1.0 - (1.0 - a) * (1.0 - b)));
                assert!(close(blend1(BlendMode::LinearDodge, a, b), a + b));
                assert!(close(blend1(BlendMode::Subtract, a, b), a - b));
                assert!(close(blend1(BlendMode::Darken, a, b), a.min(b)));
                assert!(close(blend1(BlendMode::Lighten, a, b), a.max(b)));
                assert!(close(blend1(BlendMode::Difference, a, b), (a - b).abs()));
                assert!(close(blend1(BlendMode::Exclusion, a, b), a + b - 2.0 * a * b));
            }
        }
        assert!(close(blend1(BlendMode::Overlay, 0.25, 0.3), 2.0 * 0.25 * 0.3));
        assert!(close(blend1(BlendMode::Overlay, 0.5, 0.7), 1.0 - 2.0 * 0.5 * 0.3));
        assert!(close(blend1(BlendMode::HardLight, 0.7, 0.25), 2.0 * 0.25 * 0.7));
    }

    #[test]
    fn soft_light_matches_w3c() {
        assert!(close(blend1(BlendMode::SoftLight, 0.25, 0.3), 0.25 - 0.4 * 0.25 * 0.75));
        let d = ((16.0 * 0.25 - 12.0) * 0.25 + 4.0) * 0.25;
        assert!(close(blend1(BlendMode::SoftLight, 0.25, 0.7), 0.25 + 0.4 * (d - 0.25)));
        assert!(close(blend1(BlendMode::SoftLight, 0.5, 0.7), 0.5 + 0.4 * (0.5f32.sqrt() - 0.5)));
    }

    #[test]
    fn burns_and_dodges_handle_edges() {
        assert!(close(blend1(BlendMode::ColorBurn, 1.0, 0.0), 1.0));
        assert!(close(blend1(BlendMode::ColorBurn, 0.5, 0.0), 0.0));
        assert!(close(blend1(BlendMode::ColorDodge, 0.0, 1.0), 0.0));
        assert!(close(blend1(BlendMode::ColorDodge, 0.5, 1.0), 1.0));
        assert!(close(blend1(BlendMode::ColorDodge, 0.25, 0.5), 0.5));
        for m in BlendMode::all() {
            for a in AS {
                for b in BS {
                    assert!(blend1(m, a, b).is_finite(), "{m:?} {a} {b}");
                    assert!(blend3(m, [a; 3], [b; 3]).iter().all(|v| v.is_finite()), "{m:?}");
                }
            }
        }
    }

    #[test]
    fn color_keeps_lower_luminance() {
        let a = [0.2, 0.4, 0.6];
        let out = blend3(BlendMode::Color, a, [1.0, 0.0, 0.0]);
        assert!(close(lum(out), lum(a)));
    }

    #[test]
    fn flat_normal_detail_is_identity() {
        let flat = [0.5, 0.5, 1.0];
        let n = pack([0.3, -0.2, 0.9]);
        for m in [BlendMode::NormalMapDetail, BlendMode::NormalMapCombine] {
            let out = blend3(m, n, flat);
            for i in 0..3 {
                assert!((out[i] - n[i]).abs() < 1e-4, "{m:?} {out:?} {n:?}");
            }
        }
    }

    #[test]
    fn menu_lists_each_mode_once_and_parse_round_trips() {
        let all: Vec<_> = BlendMode::all().collect();
        assert_eq!(all.len(), 32);
        for m in &all {
            assert_eq!(all.iter().filter(|x| *x == m).count(), 1);
            assert_eq!(BlendMode::parse(m.label()), Some(*m));
            assert_eq!(BlendMode::parse(&format!("{m:?}")), Some(*m));
        }
    }
}
