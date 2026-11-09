//! What one polygon fill target grows to.

use serde::{Deserialize, Serialize};

/// Polygon fill granularity: what a clicked triangle or a boxed polygon grows to
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FillMode {
    /// The triangle itself
    #[default]
    Triangle,
    /// The source polygon (quad or n-gon) the importer split into triangles
    Polygon,
    /// The UV island: triangles sharing edges in UV space
    UvIsland,
    /// The connected geometry: triangles sharing edges by position
    Mesh,
}
