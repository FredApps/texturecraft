use super::RayStart;
use texturecraft_geom::glam::Vec3;

/// One high-poly vertex seen from the transfer ray through its nearest low-poly point
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VertexReach {
    pub low_position: Vec3,
    pub start: RayStart,
    pub high_position: Vec3,
    /// The side the high poly's triangles face at the vertex, by their winding (not normalized)
    pub high_facing: Vec3,
    /// Whether the bake lets this low-poly point's ray hit the vertex's part (always, unless matching by mesh name)
    pub name_matches: bool,
}

impl VertexReach {
    /// Whether the ray misses the vertex (or may not hit its part) at `frontal` and `rear` world units of cage distance; with `ignore_backface` it also misses a vertex whose triangles face away from the ray
    pub fn missed(&self, frontal: f32, rear: f32, ignore_backface: bool) -> bool {
        let origin = self.start.origin(self.low_position, frontal);
        let (direction, front) = match self.start {
            RayStart::CageMesh(offset) => (offset.normalize_or_zero(), offset.length()),
            RayStart::Distance { up, scale } => (-up, frontal * scale),
        };
        let along_ray = (self.high_position - origin).dot(direction);

        !self.name_matches || !(0.0..=front + rear).contains(&along_ray) || (ignore_backface && self.high_facing.dot(direction) > 0.0)
    }
}
