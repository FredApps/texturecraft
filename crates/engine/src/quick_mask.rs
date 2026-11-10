//! Quick mask (Y enables, U edits, Ctrl+Shift+I inverts): a temporary texel mask per
//! texture set, painted with the normal brush while editing, that limits every paint stroke
//! while enabled. Session state only: not saved and not in the undo history.

use std::collections::BTreeMap;

use rayon::prelude::*;
use serde_json::{Value, json};
use texturecraft_core::Channel;
use texturecraft_paint::{CommitSource, commit_coverage};
use texturecraft_texel::{TileImage, from_u16, srgb_lut};

use crate::commands::{CommandSpec, always, boolp};
use crate::{Result, Session, cmd};

#[derive(Clone, Debug, Default)]
pub struct QuickMask {
    /// Strokes are limited to the mask.
    pub enabled: bool,
    /// Strokes paint the mask instead of the layer stack.
    pub editing: bool,
    /// Strokes are limited to where the mask is not painted.
    pub inverted: bool,
    /// Painted coverage per texture set (1 component).
    pub masks: BTreeMap<usize, TileImage>,
    /// Bumps on every change (overlay refresh).
    pub revision: u64,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("viewport.quickMask", "Enable quick mask", ["Viewport"], Some("Y"), "{enabled?: bool} — toggles when omitted", always, enable),
        cmd!(
            "viewport.quickMask.edit",
            "Edit quick mask",
            ["Viewport"],
            Some("U"),
            "{editing?: bool} — toggles when omitted; editing enables the mask",
            always,
            edit
        ),
        cmd!("viewport.quickMask.invert", "Invert quick mask", ["Viewport"], Some("Ctrl+Shift+I"), "{}", always, invert),
    ]
}

fn state(s: &Session) -> Value {
    let q = &s.quick_mask;
    json!({
        "enabled": q.enabled, "editing": q.editing, "inverted": q.inverted,
        "paintedTiles": q.masks.get(&s.ts()).map(TileImage::allocated_count).unwrap_or(0),
    })
}

fn changed(s: &mut Session) -> Result<Value> {
    s.quick_mask.revision = s.quick_mask.revision.wrapping_add(1);
    s.revision = s.revision.wrapping_add(1);
    Ok(state(s))
}

fn enable(s: &mut Session, p: &Value) -> Result<Value> {
    s.quick_mask.enabled = boolp(p, "enabled").unwrap_or(!s.quick_mask.enabled);
    changed(s)
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    s.cancel_stroke();
    let q = &mut s.quick_mask;
    q.editing = boolp(p, "editing").unwrap_or(!q.editing);
    q.enabled |= q.editing;
    changed(s)
}

fn invert(s: &mut Session, _: &Value) -> Result<Value> {
    s.quick_mask.inverted = !s.quick_mask.inverted;
    changed(s)
}

impl Session {
    /// The quick mask of texture set `ts` at `size` (empty when never painted).
    fn quick_mask_image(&self, ts: usize, (w, h): (u32, u32)) -> TileImage {
        match self.quick_mask.masks.get(&ts) {
            Some(m) if (m.width(), m.height()) == (w, h) => m.clone(),
            Some(m) => m.resampled(w, h, &[0]),
            None => TileImage::new(w, h, 1),
        }
    }

    /// What limits a paint stroke on `ts`: the mask and whether it is inverted (None when the
    /// quick mask is off or being edited).
    pub(crate) fn stroke_limit(&self, ts: usize, size: (u32, u32)) -> Option<(TileImage, bool)> {
        let q = &self.quick_mask;
        (q.enabled && !q.editing).then(|| (self.quick_mask_image(ts, size), q.inverted))
    }

    /// Commit a stroke into the quick mask of `ts`.
    pub(crate) fn commit_quick_mask(&mut self, ts: usize, size: (u32, u32), src: CommitSource) {
        let mut mask = self.quick_mask_image(ts, size);
        commit_coverage(&mut mask, src);
        self.quick_mask.masks.insert(ts, mask);
        self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
    }

    /// RGBA8 overlay of texture set `ts` while editing the quick mask (`size²` stretched over
    /// the set, display-encoded): the base color, tinted red where painting is blocked.
    /// Includes the stroke in progress.
    pub fn quick_mask_rgba(&self, ts: usize, size: u32) -> Option<Vec<u8>> {
        let cache = self.caches.get(ts)?;
        let (width, height) = cache.size;
        let size = size.clamp(1, width.max(height));
        let mask = self.quick_mask_image(ts, cache.size);
        let live = self.stroke.as_ref().filter(|s| s.ts == ts && s.target.is_none()).map(|s| (&s.canvas.alpha, s.stroke.settings.opacity, s.erase));
        let base = cache.composite.channels.get(&Channel::BaseColor);
        let srgb = srgb_lut();
        let inverted = self.quick_mask.inverted;
        let mut out = vec![255u8; size as usize * size as usize * 4];

        out.par_chunks_mut(size as usize * 4).enumerate().for_each(|(y, row)| {
            let ty = (y as u64 * u64::from(height) / u64::from(size)) as u32;
            for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let tx = (x as u64 * u64::from(width) / u64::from(size)) as u32;
                let mut coverage = from_u16(mask.get(tx, ty)[0]);
                if let Some((alpha, opacity, erase)) = live {
                    let stroke_coverage = from_u16(alpha.get(tx, ty)[0]) * opacity;
                    coverage = if erase { coverage * (1.0 - stroke_coverage) } else { stroke_coverage + coverage * (1.0 - stroke_coverage) };
                }

                let blocked = if inverted { coverage } else { 1.0 - coverage };
                let raw = base.map(|b| b.get(tx, ty)).unwrap_or([32768; 4]);
                let tint_amount = 0.55 * blocked;
                for (k, tint) in [0.9, 0.1, 0.15].into_iter().enumerate() {
                    let color = f32::from(srgb.get(usize::from(raw[k] >> 4)).copied().unwrap_or(0)) / 255.0;
                    if let Some(slot) = pixel.get_mut(k) {
                        *slot = ((color + (tint - color) * tint_amount).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    }
                }
            }
        });

        Some(out)
    }
}
