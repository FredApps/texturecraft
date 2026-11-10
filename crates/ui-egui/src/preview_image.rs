//! CPU renderers for shelf spheres, swatches, strokes and imported images

use egui::{Color32, ColorImage};
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::paint::alphas::{self, LibraryAlpha};
use texturecraft_engine::paint::{BrushSettings, Shape};
use texturecraft_engine::preview::{self, Sample};
use texturecraft_engine::texel::linear_to_srgb;

/// Pattern scale of the procedural swatches (repeats across the swatch)
const PROCEDURAL_SCALE: f32 = 3.0;
/// Samples per pixel along each axis of the alpha swatches
const ALPHA_SUPERSAMPLE: usize = 2;
/// Normal tilt per height step, relative to the shape's radius in pixels
const BUMP_PER_RADIUS: f32 = 0.75;
/// Exposure of the shaded previews before tone mapping
const EXPOSURE: f32 = 1.4;
/// Gap between a preview's shape and the image edge in pixels
const SHAPE_MARGIN_PX: f32 = 1.0;
/// Guards the normal division against a zero vector
const MIN_NORMAL_LENGTH: f32 = 1e-6;

/// Linear shading for view-space normal `n`, viewed along +Z
pub fn shade(n: [f32; 3], base: [f32; 3], metal: f32, rough: f32) -> [f32; 3] {
    let l = [0.45f32, 0.7, 0.55];
    let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    let l = l.map(|v| v / len);
    let ndl = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
    let ndv = n[2].max(0.0);
    // The view direction is +Z
    let h = [l[0], l[1], l[2] + 1.0];
    let hl = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
    let ndh = ((n[0] * h[0] + n[1] * h[1] + n[2] * h[2]) / hl).max(0.0);
    let shin = 2.0 / (rough * rough * rough * rough).max(1e-3) - 2.0;
    let spec_amt = ndh.powf(shin.clamp(1.0, 4096.0)) * (shin + 2.0) / 8.0;
    // Roughness blurs the reflected horizon toward the environment average
    let ry = 2.0 * ndv * n[1];
    let w = 0.06 + rough * 0.9;
    let t = ((ry + w) / (2.0 * w)).clamp(0.0, 1.0);
    let sharp = 0.04 + 0.03 * (ry + 1.0) + (0.2 + 0.55 * ry.max(0.0)) * t * t * (3.0 - 2.0 * t);
    let env = sharp + (0.28 - sharp) * (rough * rough);
    let fres = (1.0 - ndv).powi(5);
    let sky = 0.35 + 0.25 * n[1];
    let mut out = [0.0; 3];
    for k in 0..3 {
        let f0 = 0.04 + (base[k] - 0.04) * metal;
        let f = f0 + ((1.0 - rough).max(f0) - f0) * fres;
        let diffuse = base[k] * (1.0 - metal) * (ndl * 0.85 + sky * 0.5);
        out[k] = diffuse * (1.0 - f) + f * env + f0 * spec_amt * ndl * 1.5;
    }
    out
}

/// ACES filmic tone mapping of linear `x` (exposure folded in) to 0..=1
pub fn aces(x: f32) -> f32 {
    let v = x * 0.6;
    ((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14)).clamp(0.0, 1.0)
}

/// Shade row-major `size²` samples with height bumps and a transparent background
pub fn sphere_image(size: usize, samples: &[Option<Sample>]) -> ColorImage {
    let mut image = ColorImage::filled([size, size], Color32::TRANSPARENT);
    let radius = preview_radius(size);
    let height = |index: usize| samples.get(index).copied().flatten().map(|sample| sample.height);

    for (index, normal) in preview::sphere_normals(size).enumerate() {
        let (Some(normal), Some(Some(sample))) = (normal, samples.get(index)) else { continue };

        // Anti-aliased outline: opaque from one pixel inside the rim
        let coverage = ((1.0 - (normal.x * normal.x + normal.y * normal.y).sqrt()) * radius).clamp(0.0, 1.0);
        image.pixels[index] = lit_pixel(normal, height_slope(size, index, sample.height, height), radius, sample, coverage);
    }
    image
}

/// Radius in pixels of the shape filling a `size²` preview
pub(crate) fn preview_radius(size: usize) -> f32 {
    (size as f32 * 0.5 - SHAPE_MARGIN_PX).max(0.5)
}

/// Height change per pixel rightwards and upwards at `pixel`; None heights count as `center`
pub(crate) fn height_slope(size: usize, pixel: usize, center: f32, height: impl Fn(usize) -> Option<f32>) -> [f32; 2] {
    let (x, y) = (pixel % size, pixel / size);
    let at = |x: Option<usize>, y: Option<usize>| {
        let (x, y) = (x.filter(|x| *x < size)?, y?);
        height(y.checked_mul(size)?.checked_add(x)?)
    };

    // Image Y points down; the shading normal uses Y up
    let right = at(Some(x + 1), Some(y)).unwrap_or(center) - at(x.checked_sub(1), Some(y)).unwrap_or(center);
    let up = at(Some(x), y.checked_sub(1)).unwrap_or(center) - at(Some(x), Some(y + 1)).unwrap_or(center);
    [right, up]
}

/// Tilt a view-space normal by the height `slope` on a shape of `radius` pixels
pub(crate) fn bumped_normal(normal: Vec3, slope: [f32; 2], radius: f32) -> Vec3 {
    let bump = radius * BUMP_PER_RADIUS;
    let bumped = [normal.x - slope[0] * bump, normal.y - slope[1] * bump, normal.z];
    let length = (bumped[0] * bumped[0] + bumped[1] * bumped[1] + bumped[2] * bumped[2]).sqrt().max(MIN_NORMAL_LENGTH);
    Vec3::from(bumped.map(|component| component / length))
}

/// One sphere pixel of `sample`, bumped by `slope` and `coverage` opaque
fn lit_pixel(normal: Vec3, slope: [f32; 2], radius: f32, sample: &Sample, coverage: f32) -> Color32 {
    let linear = shade(bumped_normal(normal, slope, radius).to_array(), sample.base, sample.metallic, sample.roughness);
    let color = linear.map(|component| (linear_to_srgb(aces(component * EXPOSURE)) * 255.0 + 0.5) as u8);
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], (coverage * 255.0) as u8)
}

/// Grayscale `size²` swatch of procedural `kind`
pub fn procedural_image(kind: ProceduralKind, size: usize) -> ColorImage {
    use texturecraft_engine::compose::pattern::{Coords, procedural};
    use texturecraft_engine::core::{Procedural, Projection};
    use texturecraft_engine::geom::glam::{Vec2, Vec3};

    let pattern = Procedural { scale: PROCEDURAL_SCALE, ..Procedural::new(kind) };
    let mut img = ColorImage::filled([size, size], Color32::BLACK);
    for (i, px) in img.pixels.iter_mut().enumerate() {
        let uv = Vec2::new(((i % size) as f32 + 0.5) / size as f32, ((i / size) as f32 + 0.5) / size as f32);
        let v = procedural(&pattern, &Projection::default(), &Coords { uv, pos: Vec3::ZERO, nrm: Vec3::Y });
        *px = Color32::from_gray((v.clamp(0.0, 1.0) * 255.0) as u8);
    }
    img
}

/// White-on-black `size²` swatch of library alpha `alpha`
pub fn alpha_image(alpha: &LibraryAlpha, size: usize) -> ColorImage {
    let pixels = alpha.render(size, ALPHA_SUPERSAMPLE).into_iter().map(|v| Color32::from_gray((v * 255.0 + 0.5) as u8)).collect();
    ColorImage::new([size, size], pixels)
}

/// Letterbox into a `size²` square, compositing transparency over black
pub fn image_preview(img: &ImageRes, size: usize) -> ColorImage {
    let aspect = img.width as f32 / img.height.max(1) as f32;
    let (scale_u, scale_v) = if aspect >= 1.0 { (1.0, aspect) } else { (1.0 / aspect.max(1e-6), 1.0) };
    let mut preview = ColorImage::filled([size, size], Color32::BLACK);
    for y in 0..size {
        for x in 0..size {
            let u = ((x as f32 + 0.5) / size as f32 - 0.5) * scale_u + 0.5;
            let v = ((y as f32 + 0.5) / size as f32 - 0.5) * scale_v + 0.5;
            if !((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)) {
                continue;
            }

            let sample = img.sample(u, v);
            let alpha = sample[3];
            let encode = |c: f32| if img.srgb { (c.clamp(0.0, 1.0) * alpha.clamp(0.0, 1.0) * 255.0) as u8 } else { (linear_to_srgb(c * alpha) * 255.0) as u8 };
            preview.pixels[y * size + x] = Color32::from_rgb(encode(sample[0]), encode(sample[1]), encode(sample[2]));
        }
    }
    preview
}

/// Preview the brush along an S curve in an `n`-pixel square
pub fn stroke_preview(b: &BrushSettings, n: usize) -> ColorImage {
    let library = b.alpha_library.as_deref().and_then(alphas::find).map(alphas::LibraryAlpha::image);
    let radius = n as f32 * 0.11;
    let shape = Shape { alpha: b.alpha, image: None, library, level: 0 }.at_radius(radius);
    let curve = |t: f32| (n as f32 * (0.15 + 0.7 * t), n as f32 * (0.5 - 0.22 * (t * std::f32::consts::TAU).sin()));
    let mut cover = vec![0f32; n * n];
    // Stable jitter keeps the preview reproducible
    let rnd = |i: u32, k: u32| (i.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35) as f32 / u32::MAX as f32;
    let step = (b.spacing * 2.0 * radius).max(0.5);
    let (mut t, mut travelled, mut i) = (0.0f32, step, 0u32);
    let mut last = curve(0.0);
    while t <= 1.0 && i < 2000 {
        let p = curve(t);
        travelled += (p.0 - last.0).hypot(p.1 - last.1);
        last = p;
        t += 0.002;
        if travelled < step {
            continue;
        }
        travelled = 0.0;
        i += 1;
        let r = radius * (1.0 - b.jitter_size * rnd(i, 1));
        let tangent = if b.follow_path { (curve(t).1 - p.1).atan2(curve(t).0 - p.0) } else { 0.0 };
        let angle = b.angle.to_radians() + tangent + b.jitter_angle * std::f32::consts::TAU * rnd(i, 2);
        let (cx, cy) = (p.0 + b.jitter_position * radius * (rnd(i, 3) * 2.0 - 1.0), p.1 + b.jitter_position * radius * (rnd(i, 4) * 2.0 - 1.0));
        let flow = b.flow * (1.0 - b.jitter_flow * rnd(i, 5));
        let (sin, cos) = angle.sin_cos();
        let (x0, x1) = ((cx - r).floor().max(0.0) as usize, ((cx + r).ceil() as usize).min(n));
        let (y0, y1) = ((cy - r).floor().max(0.0) as usize, ((cy + r).ceil() as usize).min(n));
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = ((x as f32 + 0.5 - cx) / r, (cy - y as f32 - 0.5) / r);
                let a = shape.sample(dx * cos + dy * sin, -dx * sin + dy * cos, b.hardness, i) * flow;
                let c = &mut cover[y * n + x];
                *c = 1.0 - (1.0 - *c) * (1.0 - a.clamp(0.0, 1.0));
            }
        }
    }
    ColorImage::new([n, n], cover.into_iter().map(|v| Color32::from_gray((v * b.opacity * 255.0 + 0.5) as u8)).collect())
}
