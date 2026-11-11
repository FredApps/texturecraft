use serde::{Deserialize, Serialize};

/// Which side of the curvature map a curvature generator shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CurvatureMode {
    /// Convex edges white.
    #[default]
    Edges,
    /// Concave cavities white.
    Cavities,
    /// Edges and cavities both white.
    Both,
    /// Signed curvature around mid gray (cavities dark, edges light).
    Raw,
}

impl CurvatureMode {
    pub const ALL: [CurvatureMode; 4] = [CurvatureMode::Edges, CurvatureMode::Cavities, CurvatureMode::Both, CurvatureMode::Raw];

    pub fn label(self) -> &'static str {
        match self {
            CurvatureMode::Edges => "Edges",
            CurvatureMode::Cavities => "Cavities",
            CurvatureMode::Both => "Edges and cavities",
            CurvatureMode::Raw => "Raw curvature",
        }
    }

    /// The mode named `name` (its variant or label, any case).
    pub fn parse(name: &str) -> Option<CurvatureMode> {
        let normalized = |text: &str| text.to_ascii_lowercase().replace([' ', '_', '-'], "");
        CurvatureMode::ALL.into_iter().find(|mode| normalized(&format!("{mode:?}")) == normalized(name) || normalized(mode.label()) == normalized(name))
    }
}
