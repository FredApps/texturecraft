//! The context tool bar (brush size, flow, stroke opacity, spacing, lazy mouse, symmetry, view
//! toggles), the vertical tool shelf, the right icon rail and the status bar.

use egui::{RichText, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::Tool;

use crate::TextureCraftApp;
use crate::docking::Panel;
use crate::i18n::{tr, trf};
use crate::keymap::tip;
use crate::state::{Mode, ViewLayout};
use crate::theme::{GUTTER, Tokens, card};
use crate::widgets::{bar_slider, icon_button};

pub fn tool_icon(t: Tool) -> &'static str {
    match t {
        Tool::Paint => ic::PAINT_BRUSH,
        Tool::Eraser => ic::ERASER,
        Tool::Projection => ic::PROJECTOR_SCREEN,
        Tool::PolygonFill => ic::POLYGON,
        Tool::Smudge => ic::FINGERPRINT,
        Tool::Clone => ic::STAMP,
        Tool::ColorPicker => ic::EYEDROPPER,
        Tool::Path => ic::PATH,
        Tool::GeometryMask => ic::CUBE_FOCUS,
    }
}

pub fn paint_bar(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let tokens = Tokens::get(ui.ctx());
    egui::Panel::top("paint_bar").exact_size(40.0).show_separator_line(false).frame(egui::Frame::NONE).show(ui, |ui| {
        card(ui, &tokens, egui::Margin { bottom: GUTTER, ..Default::default() }, egui::Margin::symmetric(8, 0), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let tool = app.session.tool.tool;
                ui.label(RichText::new(tool_icon(tool)).size(15.0).color(tokens.text_strong)).on_hover_text(tool.label());
                let brush = &mut app.session.tool.brush;
                match tool {
                    Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Smudge | Tool::Clone | Tool::Path => {
                        let mut size = brush.size / 0.25 * 100.0;
                        if bar_slider(ui, tr("Size"), &mut size, 0.05..=100.0, 110.0).changed() {
                            brush.size = (size / 100.0 * 0.25).clamp(0.0005, 0.25);
                        }
                        let mut flow = brush.flow * 100.0;
                        if bar_slider(ui, tr("Flow"), &mut flow, 0.0..=100.0, 100.0).changed() {
                            brush.flow = flow / 100.0;
                        }
                        let mut opacity = brush.opacity * 100.0;
                        if bar_slider(ui, tr("Stroke opacity"), &mut opacity, 0.0..=100.0, 110.0).changed() {
                            brush.opacity = opacity / 100.0;
                        }
                        let mut spacing = brush.spacing * 100.0;
                        if bar_slider(ui, tr("Spacing"), &mut spacing, 1.0..=200.0, 100.0).changed() {
                            brush.spacing = spacing / 100.0;
                        }
                        ui.separator();
                        let lazy_on = brush.lazy > 0.0;
                        if icon_button(ui, ic::SCRIBBLE, tr("Lazy mouse"), lazy_on, 24.0).clicked() {
                            brush.lazy = if lazy_on { 0.0 } else { 20.0 };
                        }
                        if lazy_on {
                            let mut distance = brush.lazy;
                            if bar_slider(ui, tr("Distance"), &mut distance, 1.0..=200.0, 90.0).changed() {
                                brush.lazy = distance;
                            }
                        }
                        ui.separator();
                        if icon_button(ui, ic::FLIP_HORIZONTAL, tr("Symmetry (X)"), brush.symmetry.x, 24.0).clicked() {
                            brush.symmetry.x = !brush.symmetry.x;
                        }
                        if icon_button(ui, ic::FLIP_VERTICAL, tr("Symmetry (Y)"), brush.symmetry.y, 24.0).clicked() {
                            brush.symmetry.y = !brush.symmetry.y;
                        }
                        if icon_button(ui, ic::CLOUD_RAIN, tr("Particle brush: particles slide down the surface"), brush.particles.enabled, 24.0).clicked() {
                            brush.particles.enabled = !brush.particles.enabled;
                        }
                        if tool == Tool::Smudge {
                            ui.separator();
                            let mut strength = app.session.tool.smudge_strength * 100.0;
                            if bar_slider(ui, tr("Strength"), &mut strength, 0.0..=100.0, 100.0).changed() {
                                app.session.tool.smudge_strength = (strength / 100.0).clamp(0.0, 1.0);
                            }
                        }
                        if tool == Tool::Clone && app.session.tool.clone_source.is_none() {
                            ui.separator();
                            ui.label(RichText::new(tr("Hold V and click the mesh to set the clone source")).color(tokens.text_dim));
                        }
                        if tool == Tool::Path {
                            ui.separator();
                            path_controls(app, ui);
                        }
                    }
                    Tool::PolygonFill => {
                        use texturecraft_engine::FillMode;
                        for (fill_mode, icon, tip) in [
                            (FillMode::Triangle, ic::POLYGON, tr("Triangle fill")),
                            (FillMode::UvIsland, ic::SHAPES, tr("UV island fill")),
                            (FillMode::Mesh, ic::CUBE, tr("Mesh fill")),
                        ] {
                            if icon_button(ui, icon, tip, app.session.tool.fill_mode == fill_mode, 24.0).clicked() {
                                app.session.tool.fill_mode = fill_mode;
                            }
                        }
                    }
                    Tool::ColorPicker => {
                        ui.label(RichText::new(tr("Click the mesh to pick its material into the brush")).color(tokens.text_dim));
                    }
                    Tool::GeometryMask => {
                        ui.label(RichText::new(tr("Click a mesh part to show or hide the selected layer on it.")).color(tokens.text_dim));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, ic::CAMERA, tr("Screenshot of the viewport"), false, 24.0).clicked() {
                        app.toast(tr("Use the control channel or `texturecraft-cli render` for screenshots"), false);
                    }
                    ui.separator();
                    for (layout, icon, label, id) in [
                        (ViewLayout::Only2D, ic::SQUARE, "2D view only", "ui.layout.2d"),
                        (ViewLayout::Only3D, ic::CUBE, "3D view only", "ui.layout.3d"),
                        (ViewLayout::Split, ic::COLUMNS, "3D/2D views", "ui.layout.split"),
                    ] {
                        if icon_button(ui, icon, &tip(&app.ui.keymap, label, id), app.ui.layout == layout, 24.0).clicked() {
                            app.ui.layout = layout;
                        }
                    }
                    ui.separator();
                    let checker = app.session.view == texturecraft_engine::ViewMode::UvChecker;
                    if icon_button(ui, ic::CHECKERBOARD, &tip(&app.ui.keymap, tr("UV checker"), "ui.uvChecker"), checker, 24.0).clicked() {
                        let mode = if checker { "material" } else { "uvChecker" };
                        app.run("viewport.view", json!({"mode": mode}));
                    }
                });
            });
        });
    });
}

/// Path tool options and actions (top bar and properties).
pub fn path_controls(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let path = &mut app.session.tool.path;
    if icon_button(ui, ic::POLYGON, tr("Closed path"), path.closed, 24.0).clicked() {
        path.closed = !path.closed;
    }
    let fill_tip = if path.closed { tr("Fill the closed path (lasso)") } else { tr("Fill the path (close it first)") };
    if icon_button(ui, ic::PAINT_BUCKET, fill_tip, path.fill, 24.0).clicked() {
        path.fill = !path.fill;
    }
    let has_points = !path.points.is_empty();
    if ui
        .add_enabled(has_points, egui::Button::new(format!("{} {}", ic::CHECK, tr("Apply"))))
        .on_hover_text(tr("Paint the path with the brush (Enter)"))
        .clicked()
    {
        app.run("tool.path", json!({"commit": true}));
    }
    if ui.add_enabled(has_points, egui::Button::new(format!("{} {}", ic::X, tr("Cancel")))).on_hover_text(tr("Remove the path (Esc)")).clicked() {
        app.run("tool.path", json!({"clear": true}));
    }
}

pub fn tool_shelf(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let tokens = Tokens::get(ui.ctx());
    egui::Panel::left("tool_shelf").resizable(false).exact_size(40.0).show_separator_line(false).frame(egui::Frame::NONE).show(ui, |ui| {
        card(ui, &tokens, egui::Margin { left: GUTTER, right: 0, top: GUTTER, bottom: GUTTER }, egui::Margin::symmetric(3, 6), |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            for (mode, icon, label, command) in [
                (Mode::Painting, ic::PALETTE, tr("Painting"), "ui.mode.paint"),
                (Mode::Baking, ic::ATOM, tr("Bake Mesh Maps"), "ui.mode.bake"),
                (Mode::Rendering, ic::APERTURE, tr("Rendering (path traced)"), "ui.mode.render"),
            ] {
                if icon_button(ui, icon, &tip(&app.ui.keymap, label, command), app.ui.mode == mode, 28.0).clicked() {
                    crate::menus::invoke(app, &ui.ctx().clone(), command);
                }
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.add_enabled_ui(app.ui.mode == Mode::Painting, |ui| {
                for tool in Tool::ALL {
                    let selected = app.ui.mode == Mode::Painting && app.session.tool.tool == tool;
                    let hint = tip(&app.ui.keymap, tr(tool.label()), &format!("tool.{}", tool.id()));
                    if icon_button(ui, tool_icon(tool), &hint, selected, 28.0).clicked() {
                        app.run("tool.select", json!({"tool": tool.id()}));
                    }
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                let quick_mask = &app.session.quick_mask;
                for (command, icon, label, selected) in [
                    ("viewport.quickMask", ic::SELECTION_BACKGROUND, tr("Quick mask"), quick_mask.enabled),
                    ("viewport.quickMask.edit", ic::SELECTION, tr("Edit quick mask"), quick_mask.editing),
                    ("viewport.quickMask.invert", ic::SELECTION_INVERSE, tr("Invert quick mask"), quick_mask.inverted),
                ] {
                    if icon_button(ui, icon, &tip(&app.ui.keymap, label, command), selected, 28.0).clicked() {
                        app.run(command, json!({}));
                    }
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                if icon_button(ui, ic::SPHERE, tr("Add smart material from the shelf"), false, 28.0).clicked() {
                    app.ui.dock.focus(Panel::Assets);
                }
            });
        });
    });
}

pub fn rail(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let tk = Tokens::get(ui.ctx());
    egui::Panel::right("rail").resizable(false).exact_size(38.0).show_separator_line(false).frame(egui::Frame::NONE).show(ui, |ui| {
        card(ui, &tk, egui::Margin { left: 0, right: GUTTER, top: GUTTER, bottom: GUTTER }, egui::Margin::symmetric(3, 6), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if icon_button(ui, ic::EXPORT, &tip(&app.ui.keymap, tr("Export textures"), "ui.export"), app.ui.windows.export, 26.0).clicked() {
                app.ui.windows.export = !app.ui.windows.export;
            }
            for (panel, icon, id) in [
                (Panel::TextureSetSettings, ic::STACK, "ui.view.textureSetSettings"),
                (Panel::Display, ic::MONITOR, "ui.view.display"),
                (Panel::Shader, ic::SLIDERS_HORIZONTAL, "ui.view.shader"),
                (Panel::History, ic::CLOCK_COUNTER_CLOCKWISE, "ui.view.history"),
            ] {
                if icon_button(ui, icon, &tip(&app.ui.keymap, tr(panel.name()), id), app.ui.dock.is_visible(panel), 26.0).clicked() {
                    app.ui.dock.toggle(panel);
                }
            }
        });
    });
}

pub fn status_bar(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let tk = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar")
        .exact_size(22.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(tk.chrome).inner_margin(egui::Margin::symmetric(10, 0)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if let Some(p) = app.bake_progress() {
                    ui.add(egui::ProgressBar::new(p).desired_width(160.0).desired_height(10.0).text(RichText::new(tr("Baking")).size(10.0)));
                    if ui.small_button(tr("Cancel")).clicked() {
                        app.cancel_bake();
                    }
                } else if let Some(job) = &app.resize {
                    let bar = egui::ProgressBar::new(job.progress.fraction()).desired_width(160.0).desired_height(10.0);
                    ui.add(bar.animate(true).text(RichText::new(tr("Resizing")).size(10.0)));
                } else if let Some(p) = app.gpu.as_ref().and_then(|g| g.composite_progress(&app.session)) {
                    ui.add(egui::ProgressBar::new(p).desired_width(160.0).desired_height(10.0).text(RichText::new(tr("Compositing")).size(10.0)));
                    ui.ctx().request_repaint();
                } else if let Some(msg) = app.session.log.last() {
                    ui.label(RichText::new(msg).size(11.0).color(tk.text_dim));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let gpu = app.gpu.as_ref().map(|g| g.adapter.as_str()).unwrap_or(tr("no GPU"));
                    ui.label(
                        RichText::new(trf("Version: {version} ({gpu})", &[("version", &texturecraft_engine::VERSION), ("gpu", &gpu)]))
                            .size(11.0)
                            .color(tk.text_dim),
                    );
                    ui.separator();
                    let m = app.session.memory();
                    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
                    let total = mb(m.total());
                    let decimals = if total < 10.0 { 1 } else { 0 };
                    let mb1 = |b: usize| format!("{:.1}", mb(b));
                    ui.label(RichText::new(trf("Layer memory: {mb} MB", &[("mb", &format!("{total:.decimals$}"))])).size(11.0).color(tk.text_dim)).on_hover_ui(
                        |ui| {
                            ui.label(trf(
                                "Painted layer pixels {pixels} MB, composited channels {composites} MB, compositing cache {cache} MB",
                                &[("pixels", &mb1(m.pixels)), ("composites", &mb1(m.composites)), ("cache", &mb1(m.cache))],
                            ));
                        },
                    );
                    ui.separator();
                    ui.label(RichText::new(trf("UI {ms} ms", &[("ms", &format!("{:.1}", app.ui_ms))])).size(11.0).color(tk.text_dim));
                    ui.add_space(vec2(0.0, 0.0).x);
                });
            });
        });
}
