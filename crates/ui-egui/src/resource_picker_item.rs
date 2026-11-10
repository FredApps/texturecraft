//! One choice offered by a resource picker

/// A resource the picker offers: its id, translated label and group, and its thumbnail
pub(crate) struct ResourcePickerItem {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub thumbnail: Option<egui::TextureHandle>,
}
