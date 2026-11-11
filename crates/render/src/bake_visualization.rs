//! What bake mode's 3D view draws while a bake is set up: the high poly, the cage, the UV seams and the low poly

/// Which parts of the bake setup the 3D view draws
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeVisualization {
    pub high_poly: bool,
    pub cage: bool,
    pub uv_seams: bool,
    pub low_poly: bool,
    /// Draw the cage, seams and low poly of only this texture set
    pub only_set: Option<usize>,
}
