//! Shading of the smart material previews: a dark studio whose soft boxes show in reflections and light the diffuse color

use egui::{Color32, ColorImage};
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::orb_preview::MaterialPreviewPixel;
use texturecraft_engine::texel::linear_to_srgb;

use crate::preview_image::{aces, bumped_normal, height_slope, preview_radius};
use crate::soft_box::{SoftBox, smoothstep};

/// The studio's soft boxes: a key at the upper right, a fill at the left and a rim light above and behind
const SOFT_BOXES: [SoftBox; 3] = [
    SoftBox { direction: [0.55, 0.75, 0.35], radius_rad: 0.6, intensity: 2.6 },
    SoftBox { direction: [-0.95, 0.2, 0.1], radius_rad: 0.7, intensity: 0.9 },
    SoftBox { direction: [0.1, 0.9, -0.45], radius_rad: 0.35, intensity: 1.6 },
];
/// The studio's backdrop brightness straight down and straight up (linear)
const FLOOR: f32 = 0.03;
const CEILING: f32 = 0.16;
/// The lowest roughness shaded (a perfect mirror would show the soft boxes' hard edges)
const MIN_ROUGHNESS: f32 = 0.02;
/// Reflectance of dielectrics at normal incidence
const DIELECTRIC_F0: f32 = 0.04;
/// Exposure before tone mapping
const EXPOSURE: f32 = 1.4;

/// A smart material preview (sRGB, transparent off the shape) from `size²` preview pixels; height bumps the normals
pub fn material_preview_image(size: usize, pixels: &[Option<MaterialPreviewPixel>]) -> ColorImage {
    let radius = preview_radius(size);
    let height = |index: usize| pixels.get(index).copied().flatten().map(|pixel| pixel.sample.height);
    let mut image = ColorImage::filled([size, size], Color32::TRANSPARENT);

    for (index, (out, pixel)) in image.pixels.iter_mut().zip(pixels).enumerate() {
        let Some(pixel) = pixel else { continue };

        let sample = pixel.sample;
        let normal = bumped_normal(pixel.normal, height_slope(size, index, sample.height, height), radius);
        let roughness = sample.roughness.max(MIN_ROUGHNESS);
        let normal_dot_view = normal.z.max(0.0);
        // Mirror the direction towards the viewer (+Z) about the normal
        let reflected = (2.0 * normal_dot_view * normal - Vec3::Z).normalize_or_zero();
        let specular_light = reflected_light(reflected, roughness);
        let diffuse_light = diffuse_light(normal);
        // Schlick's Fresnel term, damped on rough surfaces below
        let fresnel = (1.0 - normal_dot_view).powi(5);

        let color = [0, 1, 2].map(|component| {
            let base = sample.base[component];
            let f0 = DIELECTRIC_F0 + (base - DIELECTRIC_F0) * sample.metallic;
            let reflectance = f0 + ((1.0 - roughness).max(f0) - f0) * fresnel;
            let linear = reflectance * specular_light + base * (1.0 - sample.metallic) * (1.0 - reflectance) * diffuse_light;
            (linear_to_srgb(aces(linear * EXPOSURE)) * 255.0 + 0.5) as u8
        });
        let alpha = (pixel.coverage.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        *out = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], alpha);
    }
    image
}

/// How far unit direction component `y` points up: 0 straight down, 1 straight up
fn upward(y: f32) -> f32 {
    y * 0.5 + 0.5
}

/// The backdrop's brightness `upward` of the way from the floor (0) to the ceiling (1)
fn backdrop(upward: f32) -> f32 {
    FLOOR + (CEILING - FLOOR) * upward
}

/// The studio seen along unit direction `view` by a surface of `roughness`
fn reflected_light(view: Vec3, roughness: f32) -> f32 {
    // A soft horizon: reflections sweep from floor to ceiling smoothly
    backdrop(smoothstep(upward(view.y))) + SOFT_BOXES.iter().map(|soft_box| soft_box.reflected(view, roughness)).sum::<f32>()
}

/// The light reaching a diffuse surface with unit normal `normal`
fn diffuse_light(normal: Vec3) -> f32 {
    backdrop(upward(normal.y)) + SOFT_BOXES.iter().map(|soft_box| soft_box.diffuse(normal)).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::preview::Sample;

    use super::*;

    #[test]
    fn the_studio_lights_the_shape_from_the_upper_right() {
        let polished_metal = Sample { base: [0.9; 3], metallic: 1.0, roughness: 0.3, height: 0.0 };
        let facing = |normal: Vec3| Some(MaterialPreviewPixel { sample: polished_metal, normal: normal.normalize(), coverage: 1.0 });
        // Halfway between the viewer and the key light, a mirror reflects the key
        let towards_the_key = SOFT_BOXES[0].unit_direction() + Vec3::Z;
        let pixels = [facing(towards_the_key), facing(Vec3::new(-0.3, -0.9, 0.3)), None, None];

        let image = material_preview_image(2, &pixels);
        let [lit, unlit, outside] = [0, 1, 2].map(|index| image.pixels[index]);
        assert!(lit.r() > 200 && unlit.r() < 80, "the key light's reflection shows: {lit:?} vs {unlit:?}");
        assert_eq!((lit.a(), outside.a()), (255, 0), "opaque on the shape, transparent outside");
    }
}
