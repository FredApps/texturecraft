//! A polygon fill box being dragged: what it filled so far and what it needs to refill each move.

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_core::LayerId;
use texturecraft_geom::OrbitCamera;
use texturecraft_geom::glam::Vec2;
use texturecraft_paint::EyeDepth;
use texturecraft_texel::TileImage;

use crate::session::{Document, StackTarget};
use crate::triangle_partition::TrianglePartition;

/// A polygon fill box being dragged: the fill shows live and becomes one undo step when the drag ends
pub(crate) struct PolygonFillDrag {
    /// The texture set it fills
    pub ts: usize,
    /// The selected layer and content, mask or effect being filled
    pub target: (Option<LayerId>, StackTarget),
    /// The 3D view's size in pixels, or None for a drag in the 2D view
    pub viewport: Option<Vec2>,
    /// The camera [`PolygonFillDrag::points`] and [`PolygonFillDrag::depth`] were made with
    pub camera: OrbitCamera,
    /// The document before the drag, refilled each time the selection changes
    pub before: Document,
    /// The quick mask's coverage before the drag, when the drag paints the quick mask
    pub quick_mask_before: Option<BTreeMap<usize, TileImage>>,
    /// Each vertex where the box sees it: viewport pixels in the 3D view (None behind the camera), texture coordinates in the 2D view
    pub points: Vec<Option<Vec2>>,
    /// What the camera sees (3D view, visible polygons only)
    pub depth: Option<EyeDepth>,
    /// What the fill mode fills together
    pub fill_groups: TrianglePartition,
    /// The polygons the box tests, when they differ from the fill groups
    pub polygons: Option<TrianglePartition>,
    /// The triangles filled so far
    pub filled: BTreeSet<u32>,
    /// The box last shown, lowest corner first: an unmoved box isn't selected again
    pub last_box: Option<(Vec2, Vec2)>,
    /// The fill failed (reported once): the rest of the drag fills nothing
    pub failed: bool,
}
