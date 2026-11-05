use crate::{BakeError, MAX_TEXELS};
use texturecraft_geom::glam::Vec2;

pub const MAX_DABS: usize = 200_000;

// Bound total work across all dabs before changing any pixels
pub const MAX_PAINT_TEXELS: u64 = 1 << 26;

#[derive(Clone, Debug, PartialEq)]
pub struct BakePaintMap {
    pub width: u32,
    pub height: u32,
    /// Values in 0..=1 over the texture set UV atlas
    pub values: Vec<f32>,
}

impl BakePaintMap {
    pub fn constant(value: f32) -> BakePaintMap {
        BakePaintMap { width: 1, height: 1, values: vec![value] }
    }

    pub fn validate(&self) -> Result<(), BakeError> {
        let invalid = |m: &str| Err(BakeError::Invalid(m.into()));
        let texels = u64::from(self.width) * u64::from(self.height);
        if self.width == 0 || self.height == 0 || texels > MAX_TEXELS || self.values.len() as u64 != texels {
            return invalid("the bake paint map's size doesn't match its values");
        }
        if self.values.iter().any(|v| !v.is_finite()) {
            return invalid("the bake paint map has a value that is not a finite number");
        }
        Ok(())
    }

    pub fn resized(&self, width: u32, height: u32) -> Result<BakePaintMap, BakeError> {
        let texels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || texels > MAX_TEXELS {
            return Err(BakeError::Invalid(format!("a {width} × {height} bake paint map is outside the supported size")));
        }
        let mut values = Vec::with_capacity(texels as usize);
        for y in 0..height {
            let v = (y as f32 + 0.5) / height as f32;
            for x in 0..width {
                values.push(self.sample(Vec2::new((x as f32 + 0.5) / width as f32, v)));
            }
        }
        Ok(BakePaintMap { width, height, values })
    }

    pub fn paint(&mut self, points: &[Vec2], radius: f32, value: f32, hardness: f32, strength: f32) -> Result<(), BakeError> {
        let ok = radius.is_finite() && radius > 0.0 && value.is_finite() && hardness.is_finite() && strength.is_finite();
        if !ok || points.iter().any(|p| !p.is_finite()) {
            return Err(BakeError::Invalid("the bake brush needs finite numbers and a radius above 0".into()));
        }
        self.validate()?;
        let (value, hardness, strength) = (value.clamp(0.0, 1.0), hardness.clamp(0.0, 0.999), strength.clamp(0.0, 1.0));
        let step = (radius * 0.25).max(1e-5);
        let mut dabs = Vec::new();
        for (i, p) in points.iter().enumerate() {
            match i.checked_sub(1).and_then(|j| points.get(j)) {
                Some(prev) => {
                    let n = ((*p - *prev).length() / step).ceil().clamp(1.0, MAX_DABS as f32) as usize;
                    dabs.extend((1..=n).map(|k| *prev + (*p - *prev) * (k as f32 / n as f32)));
                }
                None => dabs.push(*p),
            }
            if dabs.len() > MAX_DABS {
                return Err(BakeError::Invalid(format!("a bake stroke is limited to {MAX_DABS} dabs; use a larger radius or fewer points")));
            }
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let span = |c: Vec2| {
            let x0 = (((c.x - radius) * w as f32).floor().max(0.0)) as usize;
            let x1 = (((c.x + radius) * w as f32).ceil().min(w as f32)).max(0.0) as usize;
            let y0 = (((c.y - radius) * h as f32).floor().max(0.0)) as usize;
            let y1 = (((c.y + radius) * h as f32).ceil().min(h as f32)).max(0.0) as usize;
            (x0, x1, y0, y1)
        };
        let work: u64 = dabs
            .iter()
            .map(|c| {
                let (x0, x1, y0, y1) = span(*c);
                x1.saturating_sub(x0) as u64 * y1.saturating_sub(y0) as u64
            })
            .fold(0, u64::saturating_add);
        if work > MAX_PAINT_TEXELS {
            return Err(BakeError::Invalid(format!(
                "this stroke would visit {work} texels (limit {MAX_PAINT_TEXELS}); use a smaller radius, a smaller map or a shorter stroke"
            )));
        }
        for c in dabs {
            let (x0, x1, y0, y1) = span(c);
            for y in y0..y1 {
                let dv = (y as f32 + 0.5) / h as f32 - c.y;
                for x in x0..x1 {
                    let du = (x as f32 + 0.5) / w as f32 - c.x;
                    let t = (du * du + dv * dv).sqrt() / radius;
                    if t >= 1.0 {
                        continue;
                    }
                    let fall = if t <= hardness {
                        1.0
                    } else {
                        let k = (1.0 - t) / (1.0 - hardness);
                        k * k * (3.0 - 2.0 * k)
                    };
                    if let Some(v) = self.values.get_mut(y * w + x) {
                        *v += (value - *v) * fall * strength;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn sample(&self, uv: Vec2) -> f32 {
        let (w, h) = (self.width.max(1) as usize, self.height.max(1) as usize);
        if !uv.is_finite() {
            return 0.0;
        }
        let x = (uv.x.clamp(0.0, 1.0) * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
        let y = (uv.y.clamp(0.0, 1.0) * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let v = |xx: usize, yy: usize| self.values.get(yy * w + xx).copied().unwrap_or(0.0);
        let top = v(x0, y0) * (1.0 - fx) + v(x1, y0) * fx;
        let bottom = v(x0, y1) * (1.0 - fx) + v(x1, y1) * fx;
        (top * (1.0 - fy) + bottom * fy).clamp(0.0, 1.0)
    }
}
