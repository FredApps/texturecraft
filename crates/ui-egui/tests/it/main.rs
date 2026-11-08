//! One integration-test executable avoids linking the same dependencies for every feature.
//! Keep feature tests in their own modules; the standalone ui_snapshots.rs target remains available.

mod bake_mode;
mod brush_drag;
mod close_unsaved;
mod color_picker;
mod curvature_generator;
mod effect_rows;
mod effect_sliders;
mod fill_channel_image;
mod fill_projection_section;
mod filter_picker;
mod history_panel;
mod layer_clipboard;
mod layer_rename;
mod path_list;
mod pen_pressure;
mod polygon_fill;
mod properties_popup;
mod rail;
mod shelf_drop;
mod shelf_hover_preview;
mod shift_lines_2d;
mod skew_paint;
mod split_divider;
mod start_window;
mod toolbar;
