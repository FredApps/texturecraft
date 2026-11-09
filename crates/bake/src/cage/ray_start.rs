use texturecraft_geom::glam::Vec3;

/// Where the bake's transfer ray for one low-poly point starts
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayStart {
    /// On the cage mesh: the offset from the cage to the low-poly point
    CageMesh(Vec3),
    /// Out along `up` by the frontal distance times `scale` (the painted cage amount)
    Distance { up: Vec3, scale: f32 },
}

impl RayStart {
    /// The ray origin for the low-poly point `position` at `frontal` world units of max frontal distance
    pub fn origin(&self, position: Vec3, frontal: f32) -> Vec3 {
        match *self {
            RayStart::CageMesh(offset) => position - offset,
            RayStart::Distance { up, scale } => position + up * frontal * scale,
        }
    }
}
