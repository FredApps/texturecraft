//! Compact widgets matching the app's look: labelled slim sliders, icon buttons, section
//! headers, channel toggle chips and color swatches.

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Color32, CornerRadius, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::theme::Tokens;

/// The drag-and-drop payload of type `T` released on `resp` this frame. Unlike
/// [`Response::dnd_release_payload`] (which clears a payload of any type), it leaves payloads
/// of other types for the next drop target.
pub fn dnd_release<T: std::any::Any + Send + Sync>(resp: &Response) -> Option<std::sync::Arc<T>> {
    resp.dnd_hover_payload::<T>().and_then(|_| resp.dnd_release_payload::<T>())
}

/// A labelled slider in two rows: "Label ........ value" over a slim track with a round knob.
/// Returns the response of the track (`.changed()` while dragged); for a value kept in the
/// document, use [`slider_edit`].
pub fn slider(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, decimals: usize) -> Response {
    slider_inner(ui, label, value, range, decimals).0
}

/// [`slider`] for a value re-read from the document every frame: `Some(merge)` on every frame
/// that changes `value` (live while dragging, a click on the track, a typed value). `merge` is
/// true for the later changes of one drag, so the edit that applies them (with the command's
/// `merge` param) continues the drag's first undo step.
pub fn slider_edit(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, decimals: usize) -> Option<bool> {
    slider_inner(ui, label, value, range, decimals).1
}

fn slider_inner(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, decimals: usize) -> (Response, Option<bool>) {
    let tokens = Tokens::get(ui.ctx());
    let (minimum, maximum) = (*range.start(), *range.end());
    let width = ui.available_width();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let value_width = [minimum, maximum]
        .into_iter()
        .map(|bound| ui.painter().layout_no_wrap(format!("{bound:.decimals$}"), font.clone(), tokens.text).size().x)
        .fold(44.0, f32::max);
    let mut edit = None;
    let field = ui
        .horizontal(|ui| {
            ui.label(RichText::new(label).color(tokens.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let id = ui.next_auto_id();
                let shown = format!("{value:.decimals$}");
                // Keep unfinished input while the document value is re-read each frame
                let mut text = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_else(|| shown.clone());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(id)
                        .font(font)
                        .desired_width(value_width)
                        .align(egui::Align2::RIGHT_CENTER)
                        .frame(egui::Frame::NONE),
                );
                if response.has_focus() {
                    ui.data_mut(|data| data.insert_temp(id, text.clone()));
                } else {
                    ui.data_mut(|data| data.remove_temp::<String>(id));
                    // Escape can lose focus before the app consumes the key
                    let escape =
                        ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
                    if response.lost_focus()
                        && !escape
                        && text != shown
                        && let Ok(parsed) = text.trim().parse::<f32>()
                        && parsed.is_finite()
                        && parsed.clamp(minimum, maximum) != *value
                    {
                        *value = parsed.clamp(minimum, maximum);
                        edit = Some(false);
                    }
                }
                id
            })
            .inner
        })
        .inner;
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, 14.0), Sense::click_and_drag());
    if response.is_pointer_button_down_on() {
        ui.data_mut(|data| data.remove_temp::<String>(field));
        ui.memory_mut(|memory| memory.surrender_focus(field));
    }
    if edit.is_some() {
        response.mark_changed();
    }
    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let fraction = ((pointer.x - rect.left() - 5.0) / (rect.width() - 10.0).max(1.0)).clamp(0.0, 1.0);
        let next = minimum + (maximum - minimum) * fraction;
        if (next - *value).abs() > f32::EPSILON {
            *value = next;
            response.mark_changed();
            // Later frames of one drag share its undo step
            edit = Some(ui.data_mut(|data| std::mem::replace(data.get_temp_mut_or_default::<bool>(response.id), true)));
        }
    }
    if response.drag_stopped() || response.clicked() {
        ui.data_mut(|data| data.remove_temp::<bool>(response.id));
    }

    let fraction = if maximum > minimum { ((*value - minimum) / (maximum - minimum)).clamp(0.0, 1.0) } else { 0.0 };
    track(ui, &response, rect.left() + 5.0..=rect.right() - 5.0, rect.center().y, fraction, 2.0, 4.5);
    (response, edit)
}

/// A small inline slider for the top tool bar: "Label value" above a short track.
pub fn bar_slider(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32) -> Response {
    let tk = Tokens::get(ui.ctx());
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click_and_drag());
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let t = ((p.x - rect.left() - 4.0) / (rect.width() - 8.0).max(1.0)).clamp(0.0, 1.0);
        let v = lo + (hi - lo) * t;
        if (v - *value).abs() > f32::EPSILON {
            *value = v;
            resp.mark_changed();
        }
    }
    let p = ui.painter();
    let font = egui::FontId::proportional(11.0);
    p.text(rect.left_top() + vec2(2.0, 2.0), egui::Align2::LEFT_TOP, label, font.clone(), tk.text);
    p.text(rect.right_top() + vec2(-2.0, 2.0), egui::Align2::RIGHT_TOP, format!("{:.0}", value), font, tk.text_strong);
    let t = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    track(ui, &resp, rect.left() + 4.0..=rect.right() - 4.0, rect.bottom() - 7.0, t, 1.5, 4.0);
    resp
}

/// A slider track from `x` at height `y`: the rail, the accent up to fraction `t` and a round knob.
fn track(ui: &Ui, resp: &Response, x: std::ops::RangeInclusive<f32>, y: f32, t: f32, width: f32, knob: f32) {
    let tk = Tokens::get(ui.ctx());
    let p = ui.painter();
    let (x0, x1) = (*x.start(), *x.end());
    let xk = x0 + (x1 - x0) * t;
    p.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], Stroke::new(width, tk.button_border));
    p.line_segment([egui::pos2(x0, y), egui::pos2(xk, y)], Stroke::new(width, tk.accent));
    let ring = if resp.hovered() || resp.dragged() { tk.text_strong } else { tk.text };
    p.circle(egui::pos2(xk, y), knob, tk.panel, Stroke::new(1.5, ring));
}

/// A histogram of `counts` (buckets over 0..1, scaled to the fullest) in a field-colored well
/// the width of the panel, the part of 0..1 outside `range` dimmed.
pub fn histogram(ui: &mut Ui, counts: &[u32], range: (f32, f32)) -> Response {
    let tk = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect(rect, CornerRadius::same(tk.radius), tk.field, Stroke::new(1.0, tk.field_border), egui::StrokeKind::Inside);
    let inner = rect.shrink(3.0);
    let max = counts.iter().copied().max().unwrap_or(0).max(1) as f32;
    let w = inner.width() / counts.len().max(1) as f32;
    for (i, c) in counts.iter().enumerate() {
        let h = inner.height() * *c as f32 / max;
        let x = inner.left() + i as f32 * w;
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(x, inner.bottom() - h), egui::pos2(x + w, inner.bottom())), 0.0, tk.text_dim);
    }
    let x = |t: f32| inner.left() + inner.width() * t.clamp(0.0, 1.0);
    let shade = tk.field.gamma_multiply(0.7);
    p.rect_filled(egui::Rect::from_x_y_ranges(inner.left()..=x(range.0), inner.y_range()), 0.0, shade);
    p.rect_filled(egui::Rect::from_x_y_ranges(x(range.1)..=inner.right(), inner.y_range()), 0.0, shade);
    for t in [range.0, range.1] {
        p.line_segment([egui::pos2(x(t), inner.top()), egui::pos2(x(t), inner.bottom())], Stroke::new(1.0, tk.accent));
    }
    resp
}

/// One line of `text` cut with "…" to fit `max_width`, for list rows that shrink with their panel.
pub fn elided(painter: &egui::Painter, text: impl Into<String>, size: f32, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.into(), egui::FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    painter.layout_job(job)
}

/// A square icon button (Phosphor glyph) in a rounded tool well: `selected` sits in the accent
/// tinted well (the shared Craft icon button, `craft_ui::buttons`).
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, selected: bool, size: f32) -> Response {
    let tk = Tokens::get(ui.ctx());
    let well = |fill, icon| ButtonVisuals { fill, stroke: Stroke::NONE, icon };
    let style = IconButtonStyle {
        normal: well(Color32::TRANSPARENT, tk.text),
        hovered: well(tk.row_hover, tk.text_strong),
        pressed: Some(well(tk.tool_active, tk.text_strong)),
        selected: well(tk.tool_active, tk.text_strong),
        corner_radius: CornerRadius::same(6),
        focus_stroke: Stroke::new(1.0, tk.accent),
    };
    IconButton::new(tooltip, vec2(size, size), &style)
        .selected(selected)
        .show(ui, |ui, rect, color| {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(size * 0.62), color);
        })
        .on_hover_text(tooltip)
}

/// A flat button filled with the accent, white lettering (a dialog's main action).
pub fn primary_button<'a>(ui: &Ui, text: &str) -> egui::Button<'a> {
    let tk = Tokens::get(ui.ctx());
    egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(tk.accent_strong).stroke(Stroke::NONE)
}

/// A labelled dropdown on one row: the label, then the box filling the rest of the row, so a
/// narrow panel shortens the box (its text truncated) instead of pushing the row past its edge.
/// Returns the box's response (for a tooltip).
pub fn combo(ui: &mut Ui, label: &str, selected: impl Into<egui::WidgetText>, menu: impl FnOnce(&mut Ui)) -> Response {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .width(ui.available_width().max(60.0))
            .wrap_mode(egui::TextWrapMode::Truncate)
            .selected_text(selected)
            .show_ui(ui, menu)
            .response
    })
    .inner
}

pub(crate) fn section_chevron(ui: &Ui, is_open: bool) -> RichText {
    let chevron = if is_open {
        egui_phosphor::regular::CARET_DOWN
    } else if ui.style().right_to_left {
        egui_phosphor::regular::CARET_LEFT
    } else {
        egui_phosphor::regular::CARET_RIGHT
    };
    RichText::new(chevron).size(10.0).color(Tokens::get(ui.ctx()).text_dim)
}

/// A collapsible section header with a chevron
pub fn section(ui: &mut Ui, id: &str, title: &str, default_open: bool, body: impl FnOnce(&mut Ui)) {
    let tokens = Tokens::get(ui.ctx());
    let id = ui.make_persistent_id(id);
    let mut collapsing_state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    let response = ui
        .horizontal(|ui| {
            ui.add(egui::Label::new(section_chevron(ui, collapsing_state.is_open())).selectable(false));
            ui.add(egui::Label::new(RichText::new(title).strong().color(tokens.text_strong)).selectable(false));
        })
        .response
        .interact(Sense::click());
    if response.clicked() {
        collapsing_state.toggle(ui);
    }

    collapsing_state.show_body_unindented(ui, |ui| {
        ui.indent(id.with("in"), body);
    });
}

/// A channel chip ("color", "metal", "rough"…): toggled = the accent-tinted well.
pub fn chip(ui: &mut Ui, text: &str, on: bool) -> Response {
    let tk = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.into(), egui::FontId::proportional(11.0), tk.text);
    let size = vec2(galley.size().x + 14.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let fill = if on {
        tk.tool_active
    } else if resp.hovered() {
        tk.row_hover
    } else {
        tk.row
    };
    p.rect_filled(rect, 5.0, fill);
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(11.0), if on { tk.text_strong } else { tk.text_dim });
    resp
}

/// A color swatch button for a linear RGB value; opens a picker on click. Returns true when
/// the color changed.
pub fn color_swatch(ui: &mut Ui, id: &str, linear: &mut [f32; 3], width: f32) -> bool {
    let tk = Tokens::get(ui.ctx());
    let srgb = linear.map(|v| (texturecraft_engine::texel::linear_to_srgb(v) * 255.0 + 0.5) as u8);
    let mut c = Color32::from_rgb(srgb[0], srgb[1], srgb[2]);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click());
    ui.painter().rect(rect, 4.0, c, Stroke::new(1.0, tk.field_border), egui::StrokeKind::Inside);
    let popup = ui.make_persistent_id(id);
    if resp.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup);
    }
    let mut changed = false;
    // Open state lives in memory (`toggle_id` above); `from_response` alone shows it always.
    egui::Popup::from_response(&resp).id(popup).open_memory(None).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        // Wider than the R, G, B row at any value, so the popup keeps one size and place.
        if color_picker(ui, popup.with("picker"), &mut c, 275.0) {
            let lin = [c.r(), c.g(), c.b()].map(|v| texturecraft_engine::texel::srgb_to_linear(v as f32 / 255.0));
            *linear = lin;
            changed = true;
        }
    });
    changed
}

/// A color picker `width` wide: R, G, B (sRGB 0–255) with a copy button, a saturation/value
/// square and a hue strip. Returns true when the color changed.
fn color_picker(ui: &mut Ui, id: egui::Id, c: &mut Color32, width: f32) -> bool {
    use egui::ecolor::{hsv_from_rgb, rgb_from_hsv};
    let tk = Tokens::get(ui.ctx());
    let mut srgb = [c.r(), c.g(), c.b()];
    let hsv_of = |srgb: [u8; 3]| hsv_from_rgb(srgb.map(|v| v as f32 / 255.0));
    // Remembered with the color it was for, so hue and saturation survive black, white and grays.
    let (mut h, mut s, mut v) = ui.data(|d| d.get_temp::<(Color32, (f32, f32, f32))>(id)).filter(|(k, _)| k == c).map_or_else(|| hsv_of(srgb), |(_, hsv)| hsv);
    let mut typed = false;
    let mut dragged = false;
    ui.horizontal(|ui| {
        for (x, prefix) in srgb.iter_mut().zip(["R ", "G ", "B "]) {
            typed |= ui.add(egui::DragValue::new(x).speed(0.5).prefix(prefix)).changed();
        }
        if icon_button(ui, egui_phosphor::regular::COPY, crate::i18n::tr("Copy RGB values"), false, 20.0).clicked() {
            ui.copy_text(format!("{}, {}, {}", srgb[0], srgb[1], srgb[2]));
        }
    });
    if typed {
        (h, s, v) = hsv_of(srgb);
    }
    let border = Stroke::new(1.0, tk.field_border);
    // Saturation across, value up.
    let (rect, resp) = ui.allocate_exact_size(vec2(width, width), Sense::click_and_drag());
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        (s, v) = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0), 1.0 - ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0));
        dragged = true;
    }
    gradient(ui.painter(), rect, |x, y| rgb_from_hsv((h, x, 1.0 - y)));
    ui.painter().rect_stroke(rect, 0.0, border, egui::StrokeKind::Inside);
    let at = rect.lerp_inside(vec2(s, 1.0 - v));
    ui.painter().circle_stroke(at, 5.0, Stroke::new(2.0, Color32::WHITE));
    ui.painter().circle_stroke(at, 6.5, Stroke::new(1.0, Color32::BLACK));
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 16.0), Sense::click_and_drag());
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        h = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        dragged = true;
    }
    gradient(ui.painter(), rect, |x, _| rgb_from_hsv((x, 1.0, 1.0)));
    ui.painter().rect_stroke(rect, 0.0, border, egui::StrokeKind::Inside);
    let x = rect.left() + rect.width() * h;
    ui.painter().rect_stroke(egui::Rect::from_x_y_ranges(x - 2.0..=x + 2.0, rect.y_range()), 1.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Middle);
    if dragged {
        srgb = rgb_from_hsv((h, s, v)).map(|x| (x * 255.0 + 0.5) as u8);
    }
    let next = Color32::from_rgb(srgb[0], srgb[1], srgb[2]);
    ui.data_mut(|d| d.insert_temp(id, (next, (h, s, v))));
    let changed = next != *c;
    *c = next;
    changed
}

/// Fill `rect` with `rgb(x, y)` (sRGB 0..1 over 0..1 across and down), as a shaded mesh.
fn gradient(painter: &egui::Painter, rect: egui::Rect, rgb: impl Fn(f32, f32) -> [f32; 3]) {
    const N: u32 = 24;
    let mut mesh = egui::Mesh::default();
    for j in 0..=N {
        for i in 0..=N {
            let (x, y) = (i as f32 / N as f32, j as f32 / N as f32);
            let [r, g, b] = rgb(x, y).map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            mesh.colored_vertex(rect.lerp_inside(vec2(x, y)), Color32::from_rgb(r, g, b));
        }
    }
    for j in 0..N {
        for i in 0..N {
            let k = j * (N + 1) + i;
            mesh.add_triangle(k, k + 1, k + N + 1);
            mesh.add_triangle(k + 1, k + N + 2, k + N + 1);
        }
    }
    painter.add(mesh);
}

/// Panel title row with optional trailing content.
pub fn panel_header(ui: &mut Ui, title: &str, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        ui.label(crate::theme::caption(title));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), trailing);
    });
}
