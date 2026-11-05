use texturecraft_core::{PositionAxis, PositionMode, PositionNormalizationScale, PositionSettings};
use texturecraft_geom::{Mesh, glam::Vec3};

pub(super) struct PositionProjection {
    settings: PositionSettings,
    minimum: Vec3,
    extent: Vec3,
    center: Vec3,
    diameter: f32,
}

impl PositionProjection {
    pub(super) fn new(mesh: &Mesh, material: usize, settings: PositionSettings) -> Self {
        let initial = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        let expand = |(minimum, maximum): (Vec3, Vec3), position: &Vec3| (minimum.min(*position), maximum.max(*position));
        let (minimum, maximum) = match settings.scale {
            PositionNormalizationScale::FullScene => mesh.positions.iter().fold(initial, expand),
            PositionNormalizationScale::PerMaterial => mesh
                .triangles_of(material)
                .filter_map(|triangle| mesh.tri(triangle))
                .flatten()
                .filter_map(|index| mesh.positions.get(index))
                .fold(initial, expand),
        };
        let minimum = if minimum.is_finite() { minimum } else { Vec3::ZERO };
        let maximum = if maximum.is_finite() { maximum } else { Vec3::ZERO };
        let center = (minimum + maximum) * 0.5;
        let extent = maximum - minimum;
        Self { settings, minimum, extent, center, diameter: extent.max_element().max(1e-20) }
    }

    pub(super) fn evaluate(&self, position: Vec3) -> [f32; 3] {
        let value = match self.settings.normalization {
            PositionMode::BoundingBox => ((position - self.minimum) / self.extent.max(Vec3::splat(1e-20))).clamp(Vec3::ZERO, Vec3::ONE),
            PositionMode::BoundingSphere => ((position - self.center) / self.diameter + 0.5).clamp(Vec3::ZERO, Vec3::ONE),
            PositionMode::None => position,
        };
        match self.settings.axis {
            PositionAxis::All => value.to_array(),
            PositionAxis::X => [value.x; 3],
            PositionAxis::Y => [value.y; 3],
            PositionAxis::Z => [value.z; 3],
        }
    }
}
