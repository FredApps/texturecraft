use serde::{Deserialize, Serialize};
use texturecraft_render::BakeVisualization;

/// The Baking Visualization panel's switches in bake mode's 3D view
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BakeVisualizationToggles {
    /// The panel's eye: the whole visualization on or off
    pub on: bool,
    pub selected_set_only: bool,
    pub high_poly: bool,
    pub cage: bool,
    pub uv_seams: bool,
    pub low_poly: bool,
}

impl Default for BakeVisualizationToggles {
    fn default() -> Self {
        Self { on: true, selected_set_only: false, high_poly: true, cage: true, uv_seams: false, low_poly: true }
    }
}

impl BakeVisualizationToggles {
    /// What the renderer draws with these switches while texture set `selected_set` is selected
    pub fn visualization(&self, selected_set: usize) -> Option<BakeVisualization> {
        self.on.then_some(BakeVisualization {
            high_poly: self.high_poly,
            cage: self.cage,
            uv_seams: self.uv_seams,
            low_poly: self.low_poly,
            only_set: self.selected_set_only.then_some(selected_set),
        })
    }
}
