//! A soft box of the smart material preview studio (see [`crate::material_preview_image`])

use texturecraft_engine::geom::glam::Vec3;

/// How much roughness widens a soft box's reflection, in radians per unit of roughness
const BLUR_RAD_PER_ROUGHNESS: f32 = 1.1;
/// A reflected disc's soft edge: this share of its reflected radius plus a minimum in radians
const EDGE_SOFTNESS: f32 = 0.25;
const EDGE_SOFTNESS_MIN_RAD: f32 = 0.02;
/// The share of a soft box's light that reaches a diffuse surface facing it
const DIFFUSE_SHARE: f32 = 0.9;

/// A round area light far away, seen as a disc in reflections
pub struct SoftBox {
    /// Direction towards the light in view space (+X right, +Y up, +Z towards the viewer), not normalized
    pub direction: [f32; 3],
    /// Angular radius of the disc in radians
    pub radius_rad: f32,
    /// Brightness of the disc (linear)
    pub intensity: f32,
}

impl SoftBox {
    /// The unit direction towards the light
    pub fn unit_direction(&self) -> Vec3 {
        Vec3::from(self.direction).normalize_or_zero()
    }

    /// The light seen along unit direction `view` by a surface of `roughness`: rougher surfaces see the disc wider and dimmer (the same light spread out)
    pub fn reflected(&self, view: Vec3, roughness: f32) -> f32 {
        let reflected_radius_rad = self.radius_rad.hypot(roughness * BLUR_RAD_PER_ROUGHNESS);
        let angle_rad = view.dot(self.unit_direction()).clamp(-1.0, 1.0).acos();
        let edge_width_rad = EDGE_SOFTNESS * reflected_radius_rad + EDGE_SOFTNESS_MIN_RAD;
        let inside = smoothstep((reflected_radius_rad - angle_rad) / edge_width_rad);
        self.intensity * (self.radius_rad / reflected_radius_rad).powi(2) * inside
    }

    /// The light reaching a diffuse surface with unit normal `normal`
    pub fn diffuse(&self, normal: Vec3) -> f32 {
        self.intensity * self.radius_rad * self.radius_rad * DIFFUSE_SHARE * normal.dot(self.unit_direction()).max(0.0)
    }
}

/// Smooth 0..=1 ramp of `t` (clamped), flat at both ends
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
