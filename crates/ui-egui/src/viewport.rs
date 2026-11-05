//! The 3D viewport and the 2D UV view: navigation, painting input, brush cursor and the GPU
//! callbacks that render them.

use std::sync::Mutex;

use egui::{Color32, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use texturecraft_engine::camera_cmds::DEFAULT_CAMERA;
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::geom::glam::{Vec2, Vec3};
use texturecraft_engine::{Tool, ViewMode};
use texturecraft_render::{Frame, Shade, ViewKind};

use crate::TextureCraftApp;
use crate::gpu::ViewCallback;
use crate::i18n::{tr, trf};
use crate::state::ViewLayout;
use crate::theme::Tokens;

pub const VIEW_3D: u64 = 1;
pub const VIEW_2D: u64 = 2;

/// While on, a click in the 3D view focuses the depth of field there (a transient UI mode kept in
/// egui's memory).
fn focus_pick_id() -> egui::Id {
    egui::Id::unique("tc_focus_pick")
}

pub fn focus_picking(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(focus_pick_id())).unwrap_or(false)
}

pub fn set_focus_picking(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(focus_pick_id(), on));
}

pub fn central(app: &mut TextureCraftApp, ui: &mut Ui) {
    let mode = app.ui.mode;
    let bake_painting = mode == crate::state::Mode::Baking
        && app.ui.bake_paint_brush.on
        && !app.session.bake_meshes.high.is_empty()
        && app.session.doc.project.texture_sets.get(app.session.ts()).is_some_and(|texture_set| {
            !texture_set.bake.high_poly.use_low_as_high && texture_set.bake.cage.kind == texturecraft_engine::core::CageType::DistanceBased
        });
    app.ui.bake_paint_brush.on = bake_painting;
    if mode == crate::state::Mode::Painting && !app.ui.hide_ui {
        crate::toolbar::paint_bar(app, ui);
    }

    let tokens = Tokens::get(ui.ctx());
    let aux = match app.session.view {
        _ if mode == crate::state::Mode::Painting && app.session.quick_mask.editing => Some(crate::gpu::AuxView::QuickMask),
        ViewMode::MeshMap(m) => Some(crate::gpu::AuxView::MeshMap(m)),
        ViewMode::Mask(id) => Some(crate::gpu::AuxView::Mask(id)),
        _ => None,
    };
    let mut sync = PendingSync(Some(aux));
    let full = ui.available_rect_before_wrap();
    path_keys(app, ui);
    // Skew painting needs the 2D view alongside the 3D Bake preview
    let layout = if mode == crate::state::Mode::Painting || bake_painting { app.ui.layout } else { ViewLayout::Only3D };
    match layout {
        ViewLayout::Only3D => view_3d(app, ui, full, &mut sync),
        ViewLayout::Only2D => view_2d(app, ui, full, &mut sync),
        ViewLayout::Split => {
            let midpoint = full.center().x;
            let left = Rect::from_min_max(full.min, pos2(midpoint - 1.0, full.max.y));
            let right = Rect::from_min_max(pos2(midpoint + 1.0, full.min.y), full.max);
            view_3d(app, ui, left, &mut sync);
            view_2d(app, ui, right, &mut sync);
            ui.painter().line_segment([pos2(midpoint, full.top()), pos2(midpoint, full.bottom())], Stroke::new(2.0, tokens.chrome));
        }
    }
    if mode == crate::state::Mode::Rendering {
        crate::render_mode::show(app, ui, full);
    } else if app.render.is_active() {
        crate::render_mode::stop(app);
    }
    if mode == crate::state::Mode::Baking {
        crate::bake_mode::bake_bar(app, ui.ctx(), full);
    }
    // Keep drawing until the 3D view finishes accumulating anti-aliasing samples
    let accumulating = layout != ViewLayout::Only2D
        && app
            .render_state
            .as_ref()
            .is_some_and(|rs| rs.renderer.read().callback_resources.get::<crate::gpu::GpuState>().is_some_and(|g| g.renderer.accumulating(VIEW_3D)));
    if app.session.stroke_active() || accumulating {
        ui.ctx().request_repaint();
    }
}

fn shade(app: &TextureCraftApp) -> Shade {
    let (view_mode, channel) = match app.session.view {
        _ if app.ui.mode == crate::state::Mode::Painting && app.session.quick_mask.editing => (2, 0),
        ViewMode::Material => (0, 0),
        ViewMode::Channel(selected_channel) => (1, selected_channel.index() as u32),
        ViewMode::MeshMap(_) | ViewMode::Mask(_) => (2, 0),
        ViewMode::UvChecker => (3, 0),
    };
    let set_index = app.session.ts();
    let height_strength = app.session.doc.project.texture_sets.get(set_index).map(|texture_set| texture_set.atlas_height_strength()).unwrap_or(1.0);
    Shade {
        height_strength,
        view_mode,
        channel,
        wireframe: app.ui.uv_wireframe,
        wireframe_3d: app.ui.wireframe_3d,
        ..Shade::from_project(&app.session.doc.project)
    }
}

/// Show what the GPU side reported (each message once while its toast is up).
fn gpu_report_toasts(app: &mut TextureCraftApp, reports: crate::gpu::GpuReports) {
    let names: Vec<String> = reports.memory.iter().filter_map(|ts| app.session.doc.project.texture_sets.get(*ts).map(|t| t.name.clone())).collect();
    if !names.is_empty() {
        // One sticky notice for every set the GPU could not hold (they are not retried, so the
        // notice stays until dismissed); after a dismissal, a new one names only the new sets.
        let (sets, shown) = &mut app.gpu_memory;
        let current = shown.take().filter(|text| app.toasts.iter().any(|t| &t.text == text));
        match &current {
            Some(text) => app.toasts.retain(|t| &t.text != text),
            None => sets.clear(),
        }
        for n in names {
            if !sets.contains(&n) {
                sets.push(n);
            }
        }
        let text = match sets.as_slice() {
            [one] => trf(
                "Not enough GPU memory to show texture set \"{name}\" fully: lower the viewport Texture resolution in Settings. Painting and export are not affected.",
                &[("name", one)],
            ),
            many => {
                let short = |n: &String| match n.char_indices().nth(MAX_NAME_CHARS) {
                    Some((cut, _)) => format!("\"{}...\"", &n[..cut]),
                    None => format!("\"{n}\""),
                };
                let mut list = many.iter().take(MAX_NAMED_SETS).map(short).collect::<Vec<_>>().join(", ");
                if many.len() > MAX_NAMED_SETS {
                    list.push_str(", ...");
                }
                trf(
                    "Not enough GPU memory to show {count} texture sets fully ({names}): lower the viewport Texture resolution in Settings. Painting and export are not affected.",
                    &[("count", &many.len()), ("names", &list)],
                )
            }
        };
        app.gpu_memory.1 = Some(text.clone());
        app.toast_sticky(text);
    }
    for e in &reports.errors {
        let text = trf("GPU error: {error}", &[("error", e)]);
        if !app.toasts.iter().any(|t| t.text == text) {
            app.toast(text, true);
        }
    }
}

/// Texture sets a GPU memory notice names before "...".
const MAX_NAMED_SETS: usize = 5;
/// Characters of a set name in that notice (long names are cut, with "...").
const MAX_NAME_CHARS: usize = 32;

/// This frame's scene sync (compositing and GPU uploads), run by the first view drawn, after
/// that view handled its input: a stroke shows in the frame it is painted, not the next.
pub struct PendingSync(Option<Option<crate::gpu::AuxView>>);

fn add_callback(app: &mut TextureCraftApp, ui: &Ui, rect: Rect, frame: Frame, sync: &mut PendingSync) {
    let tk = Tokens::get(ui.ctx());
    let scene = match (sync.0.take(), app.gpu.as_mut()) {
        (Some(aux), Some(g)) => {
            let t0 = crate::now_ms();
            let scene = g.sync(&mut app.session, aux, u32::from(app.ui.prefs.viewport_texture.min(2)), f64::from(app.profiler.logic_ms));
            let reports = g.take_reports();
            app.profiler.sync_ms += (crate::now_ms() - t0) as f32;
            gpu_report_toasts(app, reports);
            // Display tiles left for later frames (the frame budget ran out).
            if app.session.caches.iter().any(|c| !c.upload.is_empty()) {
                ui.ctx().request_repaint();
            }
            Some(scene)
        }
        _ => None,
    };
    let Some(gpu) = &app.gpu else {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tr("No GPU renderer available"), egui::FontId::proportional(14.0), tk.viewport_text);
        return;
    };
    let cb = ViewCallback { frame, sync: Mutex::new(scene), rect, lost: gpu.lost.clone() };
    ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(rect, cb));
}

/// The view-mode dropdown shown at the top of each view.
fn view_header(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, id: &str) {
    let header = Rect::from_min_size(rect.min + vec2(rect.width() * 0.5 - 150.0, 8.0), vec2(300.0, 22.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(header).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let current = texturecraft_engine::camera_cmds::current(&app.session);
    let names: Vec<String> = app.session.doc.project.cameras.iter().map(|c| c.name.clone()).collect();
    let shown = current.and_then(|i| names.get(i)).map_or(tr(DEFAULT_CAMERA), String::as_str);
    egui::ComboBox::from_id_salt(format!("{id}_cam")).width(130.0).selected_text(shown).show_ui(&mut child, |ui| {
        if ui.selectable_label(current.is_none(), tr(DEFAULT_CAMERA)).clicked() {
            app.run("camera.select", json!({}));
        }
        for (i, name) in names.iter().enumerate() {
            if ui.selectable_label(current == Some(i), name).clicked() {
                app.run("camera.select", json!({"index": i}));
            }
        }
        ui.separator();
        if ui.selectable_label(false, tr("Save view as camera")).clicked() {
            app.run("camera.add", json!({}));
        }
    });
    let label = match app.session.view {
        ViewMode::MeshMap(map) => crate::i18n::baked_map_label(map).to_owned(),
        _ => app.session.view.label(),
    };
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|t| t.channel_list()).unwrap_or_default();
    egui::ComboBox::from_id_salt(format!("{id}_mode")).width(130.0).selected_text(tr(&label)).show_ui(&mut child, |ui| {
        if ui.selectable_label(app.session.view == ViewMode::Material, tr("Material")).clicked() {
            app.run("viewport.view", json!({"mode": "material"}));
        }
        ui.separator();
        for c in channels {
            if ui.selectable_label(app.session.view == ViewMode::Channel(c), tr(c.label())).clicked() {
                app.run("viewport.view", json!({"mode": c.id()}));
            }
        }
        ui.separator();
        for m in BakedMap::ALL {
            if ui.selectable_label(app.session.view == ViewMode::MeshMap(m), trf("Mesh map: {map}", &[("map", &tr(crate::i18n::baked_map_label(m)))])).clicked()
            {
                app.run("viewport.view", json!({"mode": format!("map:{}", m.id())}));
            }
        }
        ui.separator();
        if ui.selectable_label(app.session.view == ViewMode::UvChecker, tr("UV checker")).clicked() {
            app.run("viewport.view", json!({"mode": "uvChecker"}));
        }
    });
}

fn axis_gizmo(ui: &Ui, rect: Rect, app: &TextureCraftApp, uv: bool) {
    let tk = Tokens::get(ui.ctx());
    let o = pos2(rect.left() + 28.0, rect.bottom() - 28.0);
    let p = ui.painter();
    if uv {
        p.line_segment([o, o + vec2(16.0, 0.0)], Stroke::new(1.5, Color32::from_rgb(0xd8, 0x4a, 0x3a)));
        p.line_segment([o, o - vec2(0.0, 16.0)], Stroke::new(1.5, Color32::from_rgb(0x5c, 0xc0, 0x4a)));
        p.text(o + vec2(20.0, 0.0), egui::Align2::LEFT_CENTER, "U", egui::FontId::proportional(9.0), tk.viewport_text);
        p.text(o - vec2(0.0, 22.0), egui::Align2::CENTER_CENTER, "V", egui::FontId::proportional(9.0), tk.viewport_text);
        return;
    }
    let view = app.session.camera.view();
    for (axis, color, name) in [
        (Vec3::X, Color32::from_rgb(0xd8, 0x4a, 0x3a), "X"),
        (Vec3::Y, Color32::from_rgb(0x5c, 0xc0, 0x4a), "Y"),
        (Vec3::Z, Color32::from_rgb(0x4a, 0x80, 0xe0), "Z"),
    ] {
        let d = view.transform_vector3(axis);
        let end = o + vec2(d.x, -d.y) * 16.0;
        p.line_segment([o, end], Stroke::new(1.5, color));
        p.text(end + vec2(d.x, -d.y) * 6.0, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(9.0), tk.viewport_text);
    }
}

fn view_3d(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, sync: &mut PendingSync) {
    let resp = ui.interact(rect, ui.scope_id().with("view3d"), Sense::click_and_drag());
    app.viewport_rect = rect;
    let size = rect.size();
    app.session.viewport = Vec2::new(size.x.max(16.0), size.y.max(16.0));
    let (alt, shift) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
    let local = |p: egui::Pos2| Vec2::new(p.x - rect.left(), p.y - rect.top());
    let adjusting = brush_drag(app, ui, &resp);
    let navigating = !adjusting && (alt || ui.input(|i| i.pointer.middle_down() || i.pointer.secondary_down()));

    // Navigation (S + drag manipulates the projection stencil instead, below).
    let delta = ui.input(|i| i.pointer.delta());
    let s_held = app.ui.mode == crate::state::Mode::Painting && ui.input(|i| i.key_down(egui::Key::S)) && !ui.ctx().egui_wants_keyboard_input();
    let stencil_drag = s_held && app.session.tool.stencil.is_some();
    if stencil_drag || adjusting {
        // The stencil moves, turns and scales instead of the camera (handled below), or the
        // brush changes.
    } else if resp.dragged_by(PointerButton::Middle) {
        app.session.camera.pan(delta.x / size.y, delta.y / size.y);
    } else if alt && resp.dragged_by(PointerButton::Primary) {
        app.session.camera.orbit(delta.x * 0.008, delta.y * 0.008);
    } else if resp.dragged_by(PointerButton::Secondary) {
        if alt {
            app.session.camera.zoom(-delta.y * 0.01);
        } else if shift {
            // Shift + right drag turns the environment light.
            let d = &mut app.session.doc.project.display;
            d.env_rotation = (d.env_rotation + delta.x * 0.5).rem_euclid(360.0);
        } else {
            app.session.camera.orbit(delta.x * 0.008, delta.y * 0.008);
        }
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            if shift && app.ui.mode == crate::state::Mode::Painting {
                let brush = &mut app.session.tool.brush;
                brush.size = (brush.size * (1.0 + scroll * 0.002)).clamp(0.0005, 0.25);
            } else {
                app.session.camera.zoom(scroll * 0.0025);
            }
        }
    }

    // Touch screens: two fingers orbit and pinch zooms, three fingers pan. A second finger
    // cancels the stroke the first one started, so navigating leaves no paint behind.
    let touch = ui.input(|i| i.multi_touch()).filter(|_| resp.contains_pointer() || resp.dragged());
    if let Some(t) = touch {
        if app.session.stroke_active() {
            app.session.cancel_stroke();
        }
        let d = t.translation_delta;
        if t.num_touches >= 3 {
            app.session.camera.pan(d.x / size.y, d.y / size.y);
        } else {
            app.session.camera.orbit(d.x * 0.008, d.y * 0.008);
        }
        if t.zoom_delta.is_finite() && t.zoom_delta > 0.0 {
            app.session.camera.zoom(t.zoom_delta.ln());
        }
    }

    // Stencil: S + left drag rotates, S + right drag scales, S + middle drag moves.
    if s_held && let Some(st) = app.session.tool.stencil.as_mut() {
        if resp.dragged_by(PointerButton::Primary) {
            st.rotation = (st.rotation + delta.x * 0.4).rem_euclid(360.0);
        } else if resp.dragged_by(PointerButton::Secondary) {
            st.scale = (st.scale * (1.0 + delta.x * 0.005)).clamp(0.05, 10.0);
        } else if resp.dragged_by(PointerButton::Middle) {
            st.center[0] += delta.x / size.x * 2.0;
            st.center[1] -= delta.y / size.y * 2.0;
        }
    }
    // Clone source: V + click.
    let v_held = clone_source_key(app, ui);
    if v_held
        && !alt
        && (resp.clicked_by(PointerButton::Primary) || resp.drag_started_by(PointerButton::Primary))
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (q, vp) = (local(p), app.session.viewport);
        app.run("tool.cloneSource", json!({"x": q.x, "y": q.y, "viewport": [vp.x, vp.y]}));
    }
    // Depth of field focus pick: a click focuses there instead of painting.
    let focus_pick = focus_picking(ui.ctx());
    if focus_pick {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if !alt
            && resp.clicked_by(PointerButton::Primary)
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (q, vp) = (local(p), app.session.viewport);
            app.run("viewport.focusAt", json!({"x": q.x, "y": q.y, "viewport": [vp.x, vp.y]}));
            set_focus_picking(ui.ctx(), false);
        }
    }
    // Decal placement: a click moves the decal being placed there.
    let placing = app.placing_decal.filter(|_| app.ui.mode == crate::state::Mode::Painting);
    if let Some(layer) = placing {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if !alt
            && resp.clicked_by(PointerButton::Primary)
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (q, vp) = (local(p), app.session.viewport);
            app.run("layer.decal.set", json!({"layer": layer.0, "x": q.x, "y": q.y, "viewport": [vp.x, vp.y]}));
            app.placing_decal = None;
        }
    }
    // The selected decal's frame and handles: drag to move, scale or turn it.
    let decal_drag = !alt && !s_held && !v_held && !focus_pick && placing.is_none() && touch.is_none() && decal_input(app, ui, &resp, rect);
    // Painting.
    let painting = app.ui.mode == crate::state::Mode::Painting
        && !alt
        && !adjusting
        && !s_held
        && !v_held
        && !focus_pick
        && placing.is_none()
        && touch.is_none()
        && !decal_drag;
    let path_tool = app.session.tool.tool == Tool::Path;
    let vp = app.session.viewport;
    let show = |app: &TextureCraftApp, p: Vec3| project(app, rect, p);
    if painting && path_tool {
        path_input(app, ui, &resp, false, |app, p| app.session.pick(local(p), vp).map(|h| h.pos), show);
    } else if painting {
        let pressure = pen_pressure(ui);
        if resp.drag_started_by(PointerButton::Primary) || resp.clicked_by(PointerButton::Primary) {
            // Shift+click paints a straight line from where the last stroke ended.
            let line_from = if shift { app.last_paint_point } else { None };
            if let Some(p) = resp.interact_pointer_pos() {
                match app.session.begin_stroke(line_from.unwrap_or(local(p)), app.session.viewport, pressure) {
                    Ok(_) => {
                        if line_from.is_some() {
                            app.session.continue_stroke(local(p), pressure);
                        }
                        app.last_paint_point = Some(local(p));
                    }
                    Err(e) => app.toast(e.to_string(), true),
                }
            }
            if resp.clicked_by(PointerButton::Primary)
                && let Err(e) = app.session.end_stroke()
            {
                app.toast(e.to_string(), true);
            }
        } else if resp.dragged_by(PointerButton::Primary)
            && app.session.stroke_active()
            && let Some(p) = resp.interact_pointer_pos()
        {
            app.session.continue_stroke(local(p), pressure);
            app.last_paint_point = Some(local(p));
        }
        if resp.drag_stopped_by(PointerButton::Primary)
            && let Err(e) = app.session.end_stroke()
        {
            app.toast(e.to_string(), true);
        }
    }

    let frame = Frame {
        view_id: crate::viewport::VIEW_3D,
        size: [(size.x * ui.ctx().pixels_per_point()).max(1.0) as u32, (size.y * ui.ctx().pixels_per_point()).max(1.0) as u32],
        kind: ViewKind::Scene,
        camera: app.session.camera,
        shade: shade(app),
        visible: app.session.doc.project.texture_sets.iter().map(|t| t.visible).collect(),
    };
    add_callback(app, ui, rect, frame, sync);
    // Shelf drops: a material goes above the selection, an alpha on the brush.
    crate::shelf::drop_target(app, ui, &resp, crate::shelf::DropZone::View3d);

    // Brush cursor.
    if app.ui.mode == crate::state::Mode::Painting
        && !navigating
        && resp.hovered()
        && brush_tool(app.session.tool.tool)
        // While a Ctrl+drag changes the brush, its cursor stays where the drag began.
        && let Some(hp) = if adjusting { ui.input(|i| i.pointer.press_origin()) } else { resp.hover_pos() }
        && let Some(hit) = app.session.pick(local(hp), app.session.viewport)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        brush_cursor(app, ui, rect, hit.pos, hit.nrm);
        if app.session.tool.tool == Tool::Clone
            && let Some(src) = app.session.clone_source_point(Some(hit.uv))
        {
            clone_marker(app, ui, rect, src);
        }
        // Shift: preview the straight line Shift+click paints from the last stroke.
        if shift
            && !path_tool
            && let Some(from) = app.last_paint_point
        {
            rubber_band(ui, rect, pos2(rect.left() + from.x, rect.top() + from.y), hp);
        }
    }
    if app.ui.mode == crate::state::Mode::Painting && path_tool {
        path_overlay(app, ui, rect, &resp, false, show);
    }
    stencil_overlay(app, ui, rect);
    decal_overlay(app, ui, rect);
    brush_readout(app, ui, &resp, adjusting);
    if app.session.quick_mask.editing {
        ui.painter_at(rect).text(
            rect.left_top() + vec2(12.0, 40.0),
            egui::Align2::LEFT_TOP,
            tr("Editing quick mask: paint where painting is allowed (U to finish)"),
            egui::FontId::proportional(12.0),
            Color32::from_rgb(0xe8, 0x6a, 0x6a),
        );
    }
    view_header(app, ui, rect, "v3d");
    axis_gizmo(ui, rect, app, false);
}

/// What a drag on the selected decal's frame does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DecalGrab {
    /// Inside the frame: slide it over the surface.
    Move,
    /// A corner: scale it about its center.
    Scale,
    /// The round handle above its top edge: turn it.
    Rotate,
}

impl DecalGrab {
    fn cursor(self, dragging: bool) -> egui::CursorIcon {
        match self {
            DecalGrab::Move => egui::CursorIcon::Move,
            DecalGrab::Scale => egui::CursorIcon::ResizeNwSe,
            DecalGrab::Rotate if dragging => egui::CursorIcon::Grabbing,
            DecalGrab::Rotate => egui::CursorIcon::Grab,
        }
    }
}

/// A decal handle being dragged (kept in egui's memory between frames).
#[derive(Clone, Copy, Debug)]
struct DecalDrag {
    layer: texturecraft_engine::core::LayerId,
    grab: DecalGrab,
    /// Where the drag started, the decal's center on screen and the decal then.
    from: Pos2,
    center: Pos2,
    start: texturecraft_engine::core::Decal,
    /// The drag made its undo step: later updates merge into it.
    stepped: bool,
}

fn decal_drag_id() -> egui::Id {
    egui::Id::unique("tc_decal_drag")
}

/// Screen distance from the top edge to the rotation handle (points).
const DECAL_KNOB: f32 = 24.0;

/// The selected decal layer (its content, not its mask) as the 3D view shows it.
struct DecalFrame {
    layer: texturecraft_engine::core::LayerId,
    decal: texturecraft_engine::core::Decal,
    /// Top left, top right, bottom right, bottom left.
    corners: [Pos2; 4],
    center: Pos2,
    /// The middle of the top edge and the rotation handle above it.
    top: Pos2,
    knob: Pos2,
}

impl DecalFrame {
    fn of(app: &TextureCraftApp, rect: Rect) -> Option<DecalFrame> {
        use texturecraft_engine::compose::pattern::{decal_axes, decal_corners};
        use texturecraft_engine::core::{Fill, LayerKind, Projection};
        if app.ui.mode != crate::state::Mode::Painting || app.session.target != texturecraft_engine::StackTarget::Content {
            return None;
        }
        let p = &app.session.doc.project;
        let layer = app.session.selected_layer().and_then(|id| p.content_layer(id))?;
        let LayerKind::Fill(Fill { projection: Projection::Decal(decal), .. }) = &layer.kind else { return None };
        let [a, b, c, d] = decal_corners(decal)?.map(|q| project(app, rect, q));
        let corners = [a?, b?, c?, d?];
        let center = project(app, rect, decal_axes(decal)?.0)?;
        let top = corners[0].lerp(corners[1], 0.5);
        let knob = top + (top - center).normalized() * DECAL_KNOB;
        Some(DecalFrame { layer: layer.id, decal: *decal, corners, center, top, knob })
    }

    /// The handle under screen point `p`.
    fn grab(&self, p: Pos2) -> Option<DecalGrab> {
        if self.knob.distance(p) <= HANDLE_GRAB {
            return Some(DecalGrab::Rotate);
        }
        if self.corners.iter().any(|c| c.distance(p) <= HANDLE_GRAB) {
            return Some(DecalGrab::Scale);
        }
        // Inside the (convex) frame: on the same side of every edge.
        let side = |i: usize| {
            let (a, b) = (self.corners[i], self.corners[(i + 1) % 4]);
            (b - a).x * (p - a).y - (b - a).y * (p - a).x
        };
        let s = [side(0), side(1), side(2), side(3)];
        (s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)).then_some(DecalGrab::Move)
    }
}

/// Drags on the selected decal's frame and handles (one undo step per drag, the decal updates
/// live). Returns whether a decal drag owns the pointer.
fn decal_input(app: &mut TextureCraftApp, ui: &Ui, resp: &egui::Response, rect: Rect) -> bool {
    let ctx = ui.ctx();
    let frame = DecalFrame::of(app, rect);
    let mut drag = ctx.data(|d| d.get_temp::<DecalDrag>(decal_drag_id())).filter(|d| frame.as_ref().is_some_and(|f| f.layer == d.layer));
    if resp.drag_started_by(PointerButton::Primary)
        && let (Some(f), Some(from)) = (&frame, ctx.input(|i| i.pointer.press_origin()))
        && let Some(grab) = f.grab(from)
    {
        drag = Some(DecalDrag { layer: f.layer, grab, from, center: f.center, start: f.decal, stepped: false });
    }
    let Some(mut d) = drag.filter(|_| resp.dragged_by(PointerButton::Primary) || resp.drag_started_by(PointerButton::Primary)) else {
        ctx.data_mut(|m| m.remove::<DecalDrag>(decal_drag_id()));
        if let (Some(f), Some(h)) = (&frame, resp.hover_pos())
            && let Some(g) = f.grab(h)
        {
            ctx.set_cursor_icon(g.cursor(false));
        }
        return false;
    };
    ctx.set_cursor_icon(d.grab.cursor(true));
    if let Some(p) = resp.interact_pointer_pos().filter(|_| ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO)) {
        let mut params = match d.grab {
            DecalGrab::Move => {
                let q = d.center + (p - d.from) - rect.left_top();
                let vp = app.session.viewport;
                json!({"x": q.x, "y": q.y, "viewport": [vp.x, vp.y]})
            }
            DecalGrab::Scale => {
                let k = (p - d.center).length() / (d.from - d.center).length().max(1.0);
                json!({"size": [d.start.size[0] * k, d.start.size[1] * k]})
            }
            DecalGrab::Rotate => {
                // Screen y points down: counter-clockwise angles turn the decal's right toward its up.
                let angle = |v: egui::Vec2| (-v.y).atan2(v.x).to_degrees();
                json!({"rotation": d.start.rotation + angle(p - d.center) - angle(d.from - d.center)})
            }
        };
        params["layer"] = json!(d.layer.0);
        params["merge"] = json!(d.stepped);
        // Off the mesh while moving: the decal waits where it is.
        d.stepped |= texturecraft_engine::execute(&mut app.session, "layer.decal.set", &params).is_ok();
    }
    ctx.data_mut(|m| m.insert_temp(decal_drag_id(), d));
    true
}

/// The frame of the selected decal layer over the 3D view with its handles (and a hint while
/// placing it): outline, corner handles, the rotation handle and the projection direction.
fn decal_overlay(app: &mut TextureCraftApp, ui: &Ui, rect: Rect) {
    if app.ui.mode != crate::state::Mode::Painting {
        return;
    }

    let tk = Tokens::get(ui.ctx());
    use texturecraft_engine::core::{Fill, LayerKind, Projection};
    let p = &app.session.doc.project;
    let sel = app.session.selected_layer().and_then(|id| p.content_layer(id));
    let is_decal = matches!(sel.map(|l| &l.kind), Some(LayerKind::Fill(Fill { projection: Projection::Decal(_), .. })));
    // Placing follows the selection: another layer selected stops it.
    if app.placing_decal.is_some() && app.placing_decal != sel.map(|l| l.id).filter(|_| is_decal) {
        app.placing_decal = None;
    }
    let painter = ui.painter_at(rect);
    let hint_at = rect.left_top() + vec2(12.0, 40.0);
    if app.placing_decal.is_some() {
        let hint = tr("Click the mesh to place the decal");
        painter.text(hint_at, egui::Align2::LEFT_TOP, hint, egui::FontId::proportional(12.0), tk.viewport_accent);
    }
    let Some(f) = DecalFrame::of(app, rect) else { return };
    let active = ui.ctx().data(|d| d.get_temp::<DecalDrag>(decal_drag_id())).map(|d| d.grab);
    let hovered = ui.ctx().pointer_hover_pos().filter(|h| rect.contains(*h)).and_then(|h| f.grab(h));
    if app.placing_decal.is_none() && (hovered.is_some() || active.is_some()) {
        let hint = tr("Drag the frame to move the decal, a corner to scale it, the round handle to turn it");
        painter.text(hint_at, egui::Align2::LEFT_TOP, hint, egui::FontId::proportional(12.0), tk.viewport_text);
    }
    let shadow = Stroke::new(3.0, Color32::from_black_alpha(140));
    let mut outline = f.corners.to_vec();
    outline.push(f.corners[0]);
    let frame_color = if active == Some(DecalGrab::Move) || hovered == Some(DecalGrab::Move) { tk.viewport_accent } else { Color32::from_white_alpha(220) };
    for stroke in [shadow, Stroke::new(1.2, frame_color)] {
        painter.add(egui::Shape::dashed_line(&outline, stroke, 6.0, 4.0));
    }
    // The projection direction from the center.
    if let Some((o, _, _, n)) = texturecraft_engine::compose::pattern::decal_axes(&f.decal)
        && let Some(tip) = project(app, rect, o + n * f.decal.size[0].max(f.decal.size[1]) * 0.25)
    {
        painter.line_segment([f.center, tip], shadow);
        painter.line_segment([f.center, tip], Stroke::new(1.4, tk.viewport_accent));
    }
    painter.circle(f.center, 3.5, tk.viewport_accent, Stroke::new(1.0, Color32::from_black_alpha(180)));
    // Handles: white, the one under the pointer or being dragged in the accent color.
    let fill = |g: DecalGrab| if active == Some(g) || (active.is_none() && hovered == Some(g)) { tk.viewport_accent } else { Color32::WHITE };
    let edge = Stroke::new(1.0, Color32::from_black_alpha(200));
    painter.line_segment([f.top, f.knob], shadow);
    painter.line_segment([f.top, f.knob], Stroke::new(1.4, Color32::from_white_alpha(220)));
    painter.circle(f.knob, 5.0, fill(DecalGrab::Rotate), edge);
    for c in f.corners {
        painter.rect(Rect::from_center_size(c, vec2(8.0, 8.0)), 1.0, fill(DecalGrab::Scale), edge, egui::StrokeKind::Middle);
    }
}

/// Draw the projection stencil in Paint Mode
fn stencil_overlay(app: &mut TextureCraftApp, ui: &Ui, rect: Rect) {
    if app.ui.mode != crate::state::Mode::Painting || app.session.tool.tool != Tool::Projection {
        return;
    }
    let Some(stencil) = app.session.tool.stencil.clone() else { return };
    let Some(image) = app.session.images.get(&stencil.image).cloned() else { return };

    let texture = app.thumbs.stencil(ui.ctx(), &stencil.image, &image);
    let aspect = rect.width() / rect.height().max(1.0);
    let (center, half_size, rotation) = stencil.placement(image.width as f32 / image.height.max(1) as f32, aspect);
    let to_screen = |x: f32, y: f32| pos2(rect.left() + (x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - y * 0.5) * rect.height());
    let (sine, cosine) = rotation.sin_cos();
    let corner = |x: f32, y: f32| {
        let (x, y) = (x * half_size[0], y * half_size[1]);
        to_screen(center[0] + x * cosine - y * sine, center[1] + x * sine + y * cosine)
    };
    let tint = Color32::from_white_alpha((stencil.opacity * 255.0) as u8);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (x, y, u, v) in [(-1.0, 1.0, 0.0, 0.0), (1.0, 1.0, 1.0, 0.0), (1.0, -1.0, 1.0, 1.0), (-1.0, -1.0, 0.0, 1.0)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: corner(x, y), uv: pos2(u, v), color: tint });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::mesh(mesh));
    let outline = vec![corner(-1.0, 1.0), corner(1.0, 1.0), corner(1.0, -1.0), corner(-1.0, -1.0), corner(-1.0, 1.0)];
    painter.add(egui::Shape::dashed_line(&outline, Stroke::new(1.0, Color32::from_white_alpha(120)), 5.0, 4.0));
}

/// A world point on screen in the 3D view `rect` (None behind the camera).
fn project(app: &TextureCraftApp, rect: Rect, p: Vec3) -> Option<Pos2> {
    let n = app.session.camera.project(p, rect.width() / rect.height().max(1.0))?;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

/// Control point grab radius (points).
const HANDLE_GRAB: f32 = 8.0;

/// Path tool keys: Enter applies the path, Backspace removes its last point (Esc, handled with
/// the global shortcuts, cancels it).
fn path_keys(app: &mut TextureCraftApp, ui: &Ui) {
    if app.ui.mode != crate::state::Mode::Painting
        || app.session.tool.tool != Tool::Path
        || app.session.tool.path.points.is_empty()
        || ui.ctx().egui_wants_keyboard_input()
    {
        return;
    }
    let (enter, back) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)));
    if enter {
        app.run("tool.path", json!({"commit": true}));
    }
    if back {
        app.session.tool.path.points.pop();
    }
}

/// Path tool input in a view (`uv`: the 2D view). Pressing on a control point drags it,
/// elsewhere adds a point (and drags it); clicking the first point of an open path closes it.
/// `place` maps a pointer position to a path point (None off the surface), `show` a path point
/// to the screen.
fn path_input(
    app: &mut TextureCraftApp,
    ui: &Ui,
    resp: &egui::Response,
    uv: bool,
    place: impl Fn(&TextureCraftApp, Pos2) -> Option<Vec3>,
    show: impl Fn(&TextureCraftApp, Vec3) -> Option<Pos2>,
) {
    let drag_id = resp.id.with("path_drag");
    let clicked = resp.clicked_by(PointerButton::Primary);
    if (clicked || resp.drag_started_by(PointerButton::Primary))
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let path = &app.session.tool.path;
        let near = if path.uv == uv { path.points.iter().position(|p| show(app, *p).is_some_and(|s| s.distance(at) < HANDLE_GRAB)) } else { None };
        match near {
            Some(0) if clicked && !path.closed && path.points.len() >= 3 => app.session.tool.path.closed = true,
            Some(i) => ui.data_mut(|d| {
                d.insert_temp(drag_id, i);
            }),
            None => match place(app, at) {
                Some(p) => {
                    app.session.tool.path.push(p, uv);
                    let last = app.session.tool.path.points.len().saturating_sub(1);
                    ui.data_mut(|d| {
                        d.insert_temp(drag_id, last);
                    });
                }
                None => app.toast(tr("Place path points on the mesh"), false),
            },
        }
    }
    if resp.dragged_by(PointerButton::Primary)
        && let Some(i) = ui.data(|d| d.get_temp::<usize>(drag_id))
        && let Some(p) = resp.interact_pointer_pos().and_then(|pos| place(app, pos))
        && let Some(q) = app.session.tool.path.points.get_mut(i)
    {
        *q = p;
    }
    if !resp.is_pointer_button_down_on() {
        ui.data_mut(|d| d.remove::<usize>(drag_id));
    }
}

/// The path being edited: its curve, control points (the first one in the accent color) and,
/// while open, a dashed preview of the next segment.
fn path_overlay(app: &TextureCraftApp, ui: &Ui, rect: Rect, resp: &egui::Response, uv: bool, show: impl Fn(&TextureCraftApp, Vec3) -> Option<Pos2>) {
    let tk = Tokens::get(ui.ctx());
    let p = ui.painter_at(rect);
    let path = &app.session.tool.path;
    let hint = tr("Path: click adds a point, drag moves it, Enter applies, Esc cancels");
    p.text(rect.left_bottom() + vec2(56.0, -12.0), egui::Align2::LEFT_BOTTOM, hint, egui::FontId::proportional(12.0), tk.viewport_text);
    if path.uv != uv {
        return;
    }
    let handles: Vec<Pos2> = path.points.iter().filter_map(|q| show(app, *q)).collect();
    let pts: Vec<Vec2> = handles.iter().map(|h| Vec2::new(h.x, h.y)).collect();
    let curve: Vec<Pos2> = texturecraft_engine::paint::spline(&pts, path.closed, 3.0).into_iter().map(|v| pos2(v.x, v.y)).collect();
    let color = if path.closed && path.fill { tk.viewport_accent } else { Color32::from_white_alpha(230) };
    p.add(egui::Shape::line(curve.clone(), Stroke::new(3.0, Color32::from_black_alpha(140))));
    p.add(egui::Shape::line(curve, Stroke::new(1.4, color)));
    let hover = resp.hover_pos();
    if !path.closed
        && let (Some(last), Some(to)) = (handles.last(), hover)
    {
        rubber_band(ui, rect, *last, to);
    }
    for (i, h) in handles.iter().enumerate() {
        let r = if hover.is_some_and(|m| m.distance(*h) < HANDLE_GRAB) { 5.5 } else { 4.0 };
        p.circle(*h, r, if i == 0 { tk.viewport_accent } else { Color32::WHITE }, Stroke::new(1.5, Color32::from_black_alpha(180)));
    }
}

/// Dashed preview of a straight segment.
fn rubber_band(ui: &Ui, rect: Rect, from: Pos2, to: Pos2) {
    let p = ui.painter_at(rect);
    p.add(egui::Shape::dashed_line(&[from, to], Stroke::new(2.5, Color32::from_black_alpha(120)), 6.0, 4.0));
    p.add(egui::Shape::dashed_line(&[from, to], Stroke::new(1.0, Color32::from_white_alpha(200)), 6.0, 4.0));
}

/// V held with the Clone tool in painting mode: clicks set the clone source.
fn clone_source_key(app: &TextureCraftApp, ui: &Ui) -> bool {
    app.ui.mode == crate::state::Mode::Painting
        && app.session.tool.tool == Tool::Clone
        && ui.input(|i| i.key_down(egui::Key::V))
        && !ui.ctx().egui_wants_keyboard_input()
}

/// Crosshair where the Clone tool reads from.
fn clone_marker(app: &TextureCraftApp, ui: &Ui, rect: Rect, pos: Vec3) {
    let tk = Tokens::get(ui.ctx());
    let Some(c) = project(app, rect, pos) else { return };
    let p = ui.painter_at(rect);
    for (w, color) in [(3.0, Color32::from_black_alpha(140)), (1.2, tk.viewport_accent)] {
        p.circle_stroke(c, 7.0, Stroke::new(w, color));
        p.line_segment([c - vec2(11.0, 0.0), c + vec2(11.0, 0.0)], Stroke::new(w, color));
        p.line_segment([c - vec2(0.0, 11.0), c + vec2(0.0, 11.0)], Stroke::new(w, color));
    }
}

/// Tools that paint with the brush (and show its cursor).
fn brush_tool(t: Tool) -> bool {
    matches!(t, Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Smudge | Tool::Clone | Tool::Path)
}

/// Ctrl (Cmd on macOS) + drag in a view changes the brush instead of painting or navigating: a
/// right drag its size (left/right) and hardness (up/down), a left drag its flow and angle. True
/// while Ctrl is held with a brush tool and no stroke is under way.
fn brush_drag(app: &mut TextureCraftApp, ui: &Ui, resp: &egui::Response) -> bool {
    let held =
        app.ui.mode == crate::state::Mode::Painting && brush_tool(app.session.tool.tool) && !app.session.stroke_active() && ui.input(|i| i.modifiers.command);
    if held && let Some(right) = brush_drag_button(resp) {
        adjust_brush(&mut app.session.tool.brush, right, ui.input(|i| i.pointer.delta()));
    }
    held
}

/// The button of a drag on `resp`: Some(true) right, Some(false) left.
fn brush_drag_button(resp: &egui::Response) -> Option<bool> {
    let right = resp.dragged_by(PointerButton::Secondary);
    (right || resp.dragged_by(PointerButton::Primary)).then_some(right)
}

/// While a Ctrl+drag changes the brush, its values beside where the drag began (drawn over the
/// view; the tool bar and Properties show them too).
fn brush_readout(app: &TextureCraftApp, ui: &Ui, resp: &egui::Response, adjusting: bool) {
    let Some(right) = brush_drag_button(resp).filter(|_| adjusting) else { return };
    let Some(at) = ui.input(|i| i.pointer.press_origin()) else { return };
    // The pointer would cover the values; the brush cursor stays where the drag began.
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    let b = &app.session.tool.brush;
    let text = if right {
        format!("{} {:.1}   {} {:.0}", tr("Size"), b.size / 0.25 * 100.0, tr("Hardness"), b.hardness * 100.0)
    } else {
        format!("{} {:.0}   {} {:.0}°", tr("Flow"), b.flow * 100.0, tr("Angle"), b.angle)
    };
    let tk = Tokens::get(ui.ctx());
    let p = ui.painter_at(resp.rect);
    let g = p.layout_no_wrap(text, egui::FontId::proportional(12.0), tk.text_strong);
    let r = Rect::from_min_size(at + vec2(18.0, -30.0), g.size()).expand2(vec2(6.0, 3.0));
    p.rect_filled(r, 3.0, tk.panel);
    p.galley(r.min + vec2(6.0, 3.0), g, tk.text_strong);
}

/// Brush change for a Ctrl+drag by `d` points: `right` sets the size (x) and hardness (y), else
/// the flow (x) and angle (y); dragging up raises hardness and angle.
fn adjust_brush(b: &mut texturecraft_engine::paint::BrushSettings, right: bool, d: egui::Vec2) {
    if right {
        b.size = (b.size * (1.0 + d.x * 0.005)).clamp(0.0005, 0.25);
        b.hardness = (b.hardness - d.y * 0.005).clamp(0.0, 0.999);
    } else {
        b.flow = (b.flow + d.x * 0.005).clamp(0.0, 1.0);
        b.angle = (b.angle - d.y * 0.5 + 180.0).rem_euclid(360.0) - 180.0;
    }
}

fn brush_cursor(app: &TextureCraftApp, ui: &Ui, rect: Rect, pos: Vec3, nrm: Vec3) {
    let (lo, hi) = app.session.mesh.bounds();
    let r = app.session.tool.brush.size * (hi - lo).length();
    let t = nrm.any_orthonormal_vector();
    let b = nrm.cross(t);
    let to_screen = |p: Vec3| project(app, rect, p);
    let mut pts = Vec::with_capacity(49);
    for k in 0..=48 {
        let a = k as f32 / 48.0 * std::f32::consts::TAU;
        if let Some(s) = to_screen(pos + (t * a.cos() + b * a.sin()) * r) {
            pts.push(s);
        }
    }
    let p = ui.painter_at(rect);
    p.add(egui::Shape::line(pts.clone(), Stroke::new(2.5, Color32::from_black_alpha(140))));
    p.add(egui::Shape::line(pts, Stroke::new(1.2, Color32::from_white_alpha(230))));
    let h = app.session.tool.brush.hardness;
    if h > 0.05 {
        let mut inner = Vec::new();
        for k in 0..=48 {
            let a = k as f32 / 48.0 * std::f32::consts::TAU;
            if let Some(s) = to_screen(pos + (t * a.cos() + b * a.sin()) * r * h) {
                inner.push(s);
            }
        }
        p.add(egui::Shape::dashed_line(&inner, Stroke::new(1.0, Color32::from_white_alpha(120)), 3.0, 3.0));
    }
    if let Some(c) = to_screen(pos) {
        p.circle_filled(c, 1.5, Color32::WHITE);
    }
}

/// This frame's pen pressure (0..1), or 1 without a pen. Pressure arrives as touch force: the
/// windowing layer reports it for Windows Ink pens, and in the browser for pens that send touch
/// events (tablets and phones); other pens come through as a mouse.
/// shortcut: the windowing layer reports no pen tilt, so tilt can't drive the brush angle yet.
fn pen_pressure(ui: &Ui) -> f32 {
    let id = egui::Id::unique("pen_pressure");
    let touch = ui.input(|i| {
        i.raw.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { phase, force, .. } => Some((*phase, force.filter(|f| f.is_finite()))),
            _ => None,
        })
    });
    match touch {
        Some((_, Some(f))) => {
            let f = f.clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(id, f));
            f
        }
        // A pen lifts with no pressure, and a tap paints on that frame: keep its contact pressure.
        Some((egui::TouchPhase::End, None)) => ui.data(|d| d.get_temp(id)).unwrap_or(1.0),
        // A touch without pressure (a finger) must not inherit an earlier pen's.
        Some((egui::TouchPhase::Start, None)) => {
            ui.data_mut(|d| d.remove_temp::<f32>(id));
            1.0
        }
        _ => 1.0,
    }
}

fn view_2d(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, sync: &mut PendingSync) {
    let tk = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, ui.scope_id().with("view2d"), Sense::click_and_drag());
    let size = rect.size();
    let alt = ui.input(|i| i.modifiers.alt);
    let adjusting = brush_drag(app, ui, &resp);
    let delta = ui.input(|i| i.pointer.delta());
    if resp.dragged_by(PointerButton::Middle) || (alt && resp.dragged_by(PointerButton::Primary)) {
        app.ui.uv_pan[0] += delta.x / size.x * 2.0;
        app.ui.uv_pan[1] -= delta.y / size.y * 2.0;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old = app.ui.uv_zoom;
            let new = (old * (1.0 + scroll * 0.002)).clamp(0.1, 40.0);
            // Zoom about the cursor.
            if let Some(hp) = resp.hover_pos() {
                let c = [(hp.x - rect.left()) / size.x * 2.0 - 1.0, 1.0 - (hp.y - rect.top()) / size.y * 2.0];
                for k in 0..2 {
                    app.ui.uv_pan[k] = c[k] - (c[k] - app.ui.uv_pan[k]) * new / old;
                }
            }
            app.ui.uv_zoom = new;
        }
    }
    let ts = app.session.ts();
    let set = app.session.doc.project.texture_sets.get(ts);
    let [sx, sy] = texturecraft_render::uv_view_scale(size.x / size.y.max(1.0), app.ui.uv_zoom, set.map_or((1, 1), |t| t.size()));
    let pan = app.ui.uv_pan;
    let to_uv = |p: egui::Pos2| {
        let xc = (p.x - rect.left()) / size.x * 2.0 - 1.0;
        let yc = 1.0 - (p.y - rect.top()) / size.y * 2.0;
        Vec2::new(((xc - pan[0]) / sx + 1.0) * 0.5, (1.0 - (yc - pan[1]) / sy) * 0.5)
    };
    let to = |u: f32, v: f32| {
        let x = (u * 2.0 - 1.0) * sx + pan[0];
        let y = (1.0 - v * 2.0) * sy + pan[1];
        pos2(rect.left() + (x * 0.5 + 0.5) * size.x, rect.top() + (0.5 - y * 0.5) * size.y)
    };
    let show = |_: &TextureCraftApp, p: Vec3| Some(to(p.x, p.y));
    let path_tool = app.session.tool.tool == Tool::Path;
    // Clone source (V + click) and painting in UV space.
    let v_held = clone_source_key(app, ui);
    if v_held
        && (resp.clicked_by(PointerButton::Primary) || resp.drag_started_by(PointerButton::Primary))
        && let Some(p) = resp.interact_pointer_pos()
    {
        let uv = to_uv(p);
        app.run("tool.cloneSource", json!({"uv": [uv.x, uv.y]}));
    }
    let bake_paint_on = app.ui.mode == crate::state::Mode::Baking && app.ui.bake_paint_brush.on && !alt;
    if bake_paint_on {
        bake_paint_input(app, ui, &resp, &to_uv);
    }
    let painting = app.ui.mode == crate::state::Mode::Painting && !alt && !adjusting && !v_held;
    if painting && path_tool {
        path_input(app, ui, &resp, true, |_, p| Some(to_uv(p).extend(0.0)), show);
    } else if painting {
        let pressure = pen_pressure(ui);
        if resp.drag_started_by(PointerButton::Primary) || resp.clicked_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos()
                && let Err(e) = app.session.begin_stroke_uv(to_uv(p), pressure)
            {
                app.toast(e.to_string(), true);
            }
            if resp.clicked_by(PointerButton::Primary)
                && let Err(e) = app.session.end_stroke()
            {
                app.toast(e.to_string(), true);
            }
        } else if resp.dragged_by(PointerButton::Primary)
            && app.session.stroke_active()
            && let Some(p) = resp.interact_pointer_pos()
        {
            app.session.continue_stroke_uv(to_uv(p), pressure);
        }
        if resp.drag_stopped_by(PointerButton::Primary)
            && let Err(e) = app.session.end_stroke()
        {
            app.toast(e.to_string(), true);
        }
    }
    if resp.double_clicked() && !(painting && path_tool) {
        app.ui.uv_zoom = 1.0;
        app.ui.uv_pan = [0.0, 0.0];
    }
    let frame = Frame {
        view_id: VIEW_2D,
        size: [(size.x * ui.ctx().pixels_per_point()).max(1.0) as u32, (size.y * ui.ctx().pixels_per_point()).max(1.0) as u32],
        kind: ViewKind::Uv { ts, zoom: app.ui.uv_zoom, pan: app.ui.uv_pan },
        camera: app.session.camera,
        shade: shade(app),
        visible: app.session.doc.project.texture_sets.iter().map(|t| t.visible).collect(),
    };
    add_callback(app, ui, rect, frame, sync);
    crate::shelf::drop_target(app, ui, &resp, crate::shelf::DropZone::View2d);
    // The UV tile outlines (numbered for UDIM sets) and the path being edited.
    let painter = ui.painter_at(rect);
    let set = app.session.doc.project.texture_sets.get(ts);
    let outline = |r: Rect| painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(40)), egui::StrokeKind::Outside);
    match set.and_then(|t| Some((t, t.tile_grid()?))) {
        Some((t, g)) => {
            for (udim, (bx, by)) in t.uv_tiles.iter().filter_map(|u| Some((*u, g.block(*u)?))) {
                // A tile smaller than its block fills the block's top-left corner.
                let (u0, v0, k) = (bx as f32 / g.cols as f32, by as f32 / g.rows as f32, t.tile_scale(udim));
                let r = Rect::from_two_pos(to(u0, v0), to(u0 + k / g.cols as f32, v0 + k / g.rows as f32));
                outline(r);
                painter.text(r.left_top() + vec2(6.0, 4.0), egui::Align2::LEFT_TOP, udim.to_string(), egui::FontId::proportional(11.0), tk.viewport_text);
            }
        }
        None => {
            outline(Rect::from_two_pos(to(0.0, 0.0), to(1.0, 1.0)));
        }
    }
    if app.ui.mode == crate::state::Mode::Painting && path_tool {
        path_overlay(app, ui, rect, &resp, true, show);
    }
    if app.ui.mode == crate::state::Mode::Baking {
        bake_paint_overlay(app, ui, &painter, to(0.0, 1.0), to(1.0, 0.0));
        if bake_paint_on && let Some(hover) = resp.hover_pos() {
            let r = (to(app.ui.bake_paint_brush.radius, 0.0).x - to(0.0, 0.0).x).abs().max(2.0);
            painter.circle_stroke(hover, r, Stroke::new(1.0, Color32::from_white_alpha(200)));
        }
    }
    brush_readout(app, ui, &resp, adjusting);
    view_header(app, ui, rect, "v2d");
    axis_gizmo(ui, rect, app, true);
    let name = app.session.doc.project.texture_sets.get(ts).map(|t| t.name.clone()).unwrap_or_default();
    ui.painter().text(rect.right_bottom() + vec2(-10.0, -10.0), egui::Align2::RIGHT_BOTTOM, name, egui::FontId::proportional(11.0), tk.viewport_text);
    let _ = (RichText::new(""), Channel::BaseColor);
}

/// Paint the skew map under the pointer in the 2D view (a click is one dab, a drag a stroke).
fn bake_paint_input(app: &mut TextureCraftApp, ui: &Ui, resp: &egui::Response, to_uv: &dyn Fn(egui::Pos2) -> Vec2) {
    let last = ui.scope_id().with(("bake_paint_last", app.session.ts(), app.ui.bake_paint_brush.target as u8));
    let brush = app.ui.bake_paint_brush;
    let started = resp.drag_started_by(PointerButton::Primary) || resp.clicked_by(PointerButton::Primary);
    if !(started || resp.dragged_by(PointerButton::Primary)) {
        return;
    }
    let Some(p) = resp.interact_pointer_pos() else { return };
    let uv = to_uv(p);
    let previous: Option<[f32; 2]> = if started { None } else { ui.data(|d| d.get_temp(last)) };
    let points: Vec<[f32; 2]> = previous.into_iter().chain([[uv.x, uv.y]]).collect();
    let command = match brush.target {
        texturecraft_engine::core::BakePaintTarget::Skew => "bake.skew.paint",
        texturecraft_engine::core::BakePaintTarget::Cage => "bake.cage.paint",
    };
    if app.run(command, json!({"points": points, "radius": brush.radius, "value": brush.value, "hardness": brush.hardness, "continue": !started})).is_some() {
        ui.data_mut(|d| d.insert_temp(last, [uv.x, uv.y]));
    }
}

/// The skew map as a translucent orange layer over the UV square from `top_left` to `bottom_right`.
fn bake_paint_overlay(app: &TextureCraftApp, ui: &Ui, painter: &egui::Painter, top_left: egui::Pos2, bottom_right: egui::Pos2) {
    let Some(k) = app.session.doc.bake_paint.get(&(app.session.ts(), app.ui.bake_paint_brush.target)).filter(|k| k.values.len() > 1) else { return };
    let key = (app.session.revision, std::sync::Arc::as_ptr(k) as usize);
    let id = ui.scope_id().with("bake_paint_overlay_texture");
    let cached: Option<((u64, usize), egui::TextureHandle)> = ui.data(|d| d.get_temp(id));
    let tex = match cached {
        Some((c, t)) if c == key => t,
        _ => {
            // At most 256 texels a side: the overlay is a guide, not the map.
            let (w, h) = (k.width as usize, k.height as usize);
            let (ow, oh) = (w.min(256), h.min(256));
            let mut rgba = Vec::with_capacity(ow * oh * 4);
            for y in 0..oh {
                for x in 0..ow {
                    let v = k.values.get((y * h / oh) * w + x * w / ow).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let v = match app.ui.bake_paint_brush.target {
                        texturecraft_engine::core::BakePaintTarget::Skew => v,
                        texturecraft_engine::core::BakePaintTarget::Cage => 1.0 - v,
                    };
                    rgba.extend_from_slice(&[255, 140, 40, (v * 170.0) as u8]);
                }
            }
            let tex = ui.ctx().load_texture("bake_paint_overlay", egui::ColorImage::from_rgba_unmultiplied([ow, oh], &rgba), egui::TextureOptions::NEAREST);
            ui.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
            tex
        }
    };
    painter.image(tex.id(), Rect::from_two_pos(top_left, bottom_right), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}
