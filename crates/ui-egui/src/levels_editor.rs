//! The levels editor: draggable handles over a histogram and an output gradient, double-click to type a value

use egui::{Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Response, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};
use texturecraft_engine::core::Levels;

use crate::levels_handle::LevelsHandle;
use crate::theme::Tokens;

const HISTOGRAM_HEIGHT: f32 = 56.0;
const HANDLE_HEIGHT: f32 = 11.0;
const HANDLE_HALF_WIDTH: f32 = 5.0;

/// Height of a handle's straight sides; the rest is its tip pointing at the track
const HANDLE_SIDE_HEIGHT: f32 = 6.0;

/// Extra width on each side of a handle that still grabs it
const HANDLE_GRAB_MARGIN: f32 = 2.0;

const GRADIENT_HEIGHT: f32 = 8.0;
const GAP_BELOW_HISTOGRAM: f32 = 3.0;
const VALUE_ROW_HEIGHT: f32 = 16.0;
const VALUE_FONT_SIZE: f32 = 10.5;

/// Width of the box a value is typed into
const TYPED_VALUE_WIDTH: f32 = 52.0;

/// Padding of the histogram well around its bars
const WELL_PADDING: f32 = 3.0;

/// Gray of the gamma handle, between the black input-low and white input-high handles
const GAMMA_HANDLE_GRAY: u8 = 128;

/// Grid lines across the histogram, at these fractions of its width
const GRID_FRACTIONS: [f32; 3] = [0.25, 0.5, 0.75];

/// Decimals of a value typed in or printed beside a handle (printed without trailing zeros)
const VALUE_DECIMALS: usize = 3;

/// Space kept around a printed value; a value closer to one already printed is left out
const VALUE_SPACING: f32 = 2.0;

/// Text margin inside the box a value is typed into
const TYPED_VALUE_MARGIN: egui::Vec2 = vec2(2.0, 1.0);

/// Opacity of the shade over the histogram outside the input range
const OUTSIDE_RANGE_SHADE: f32 = 0.7;

/// Opacity of the line at gamma, fainter than the input range's accent lines
const GAMMA_LINE_OPACITY: f32 = 0.6;

/// Edits `levels` over histogram `counts`; `Some(merge)` per change, `merge` true for a drag's later frames
pub fn levels_editor(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, counts: Option<&[u32]>, levels: &mut Levels) -> Option<bool> {
    let height = VALUE_ROW_HEIGHT * 2.0 + HANDLE_HEIGHT * 2.0 + HISTOGRAM_HEIGHT + GAP_BELOW_HISTOGRAM + GRADIENT_HEIGHT;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let histogram_top = rect.top() + VALUE_ROW_HEIGHT + HANDLE_HEIGHT;
    let gradient_top = output_handles_top(rect) - GRADIENT_HEIGHT;
    let track = track(rect);
    let id = ui.make_persistent_id(("levels_editor", id_salt));

    // Gamma goes last so it stays grabbable between input handles that close in on it
    let handles = [LevelsHandle::InputLow, LevelsHandle::InputHigh, LevelsHandle::Gamma, LevelsHandle::OutputLow, LevelsHandle::OutputHigh];
    let mut edit = None;
    let mut responses = Vec::with_capacity(handles.len());
    for handle in handles {
        let x = track_x(&track, handle.position(levels));
        let top = handle_top(rect, handle);
        let grab_half_width = HANDLE_HALF_WIDTH + HANDLE_GRAB_MARGIN;
        let hit = Rect::from_x_y_ranges(x - grab_half_width..=x + grab_half_width, top..=top + HANDLE_HEIGHT);
        let response = ui.interact(hit, id.with(handle), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
        if response.drag_started() {
            ui.data_mut(|data| data.remove_temp::<bool>(response.id));
        }

        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let mut next = *levels;
            handle.set_position(&mut next, track_position(&track, pointer.x));
            if next != *levels {
                *levels = next;
                edit = Some(continues_drag(ui, response.id));
            }
        }

        let value = handle.value(levels);
        response.widget_info(|| WidgetInfo::slider(true, f64::from(value), handle.label()));
        responses.push((handle, response));
    }

    let typing_id = id.with("typing");
    let double_clicked = responses.iter().find(|(_, response)| response.double_clicked()).map(|(handle, _)| *handle);
    if let Some(handle) = double_clicked {
        start_typing(ui, typing_id, handle, levels);
    }

    let typing = ui.data(|data| data.get_temp::<(LevelsHandle, String)>(typing_id));

    let well = Rect::from_x_y_ranges(rect.x_range().shrink(HANDLE_HALF_WIDTH), histogram_top..=histogram_top + HISTOGRAM_HEIGHT);
    paint_histogram(ui, well, counts.unwrap_or_default(), levels);
    paint_output_gradient(ui, Rect::from_x_y_ranges(track, gradient_top..=gradient_top + GRADIENT_HEIGHT));
    paint_handles_and_values(ui, rect, &responses, levels, typing.as_ref().map(|(handle, _)| *handle));
    let typed = typing.and_then(|(handle, text)| typed_value(ui, rect, typing_id, (handle, text), double_clicked.is_some(), levels));
    edit.or(typed)
}

fn handle_top(rect: Rect, handle: LevelsHandle) -> f32 {
    if LevelsHandle::INPUT.contains(&handle) { rect.top() + VALUE_ROW_HEIGHT } else { output_handles_top(rect) }
}

fn output_handles_top(rect: Rect) -> f32 {
    rect.bottom() - VALUE_ROW_HEIGHT - HANDLE_HEIGHT
}

/// The x range the handles move along in the editor's `rect`, lined up with the histogram bars
fn track(rect: Rect) -> egui::Rangef {
    rect.x_range().shrink(HANDLE_HALF_WIDTH + WELL_PADDING)
}

fn track_x(track: &egui::Rangef, position: f32) -> f32 {
    track.min + track.span() * position.clamp(0.0, 1.0)
}

fn track_position(track: &egui::Rangef, x: f32) -> f32 {
    ((x - track.min) / track.span().max(1.0)).clamp(0.0, 1.0)
}

/// True once the drag of `id` has already changed a value, so its later changes share that undo step
fn continues_drag(ui: &Ui, id: Id) -> bool {
    ui.data_mut(|data| std::mem::replace(data.get_temp_mut_or_default::<bool>(id), true))
}

/// `value` with [`VALUE_DECIMALS`] decimals, trailing zeros dropped ("1", "0.25", "0.634")
fn shown_value(value: f32) -> String {
    let fixed = format!("{value:.VALUE_DECIMALS$}");
    fixed.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Where `handle`'s value of `size` goes: over an input handle, under an output one, inside `rect`
fn value_rect(rect: Rect, handle: LevelsHandle, levels: &Levels, size: egui::Vec2) -> Rect {
    let x = track_x(&track(rect), handle.position(levels));
    let top = handle_top(rect, handle);
    let placed = if LevelsHandle::INPUT.contains(&handle) {
        egui::Align2::CENTER_BOTTOM.anchor_size(pos2(x, top), size)
    } else {
        egui::Align2::CENTER_TOP.anchor_size(pos2(x, top + HANDLE_HEIGHT), size)
    };
    let shift = (rect.left() - placed.left()).max(0.0) + (rect.right() - placed.right()).min(0.0);
    placed.translate(vec2(shift, 0.0))
}

/// The histogram in its well: quarter lines, the outside of the input range dimmed, lines at the handles
fn paint_histogram(ui: &Ui, well: Rect, counts: &[u32], levels: &Levels) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter_at(well);
    painter.rect(well, CornerRadius::same(tokens.radius), tokens.field, Stroke::new(1.0, tokens.field_border), egui::StrokeKind::Inside);

    let inner = well.shrink(WELL_PADDING);
    let vertical_line = |x: f32, color: Color32| painter.line_segment([pos2(x, inner.top()), pos2(x, inner.bottom())], Stroke::new(1.0, color));
    for fraction in GRID_FRACTIONS {
        vertical_line(track_x(&inner.x_range(), fraction), tokens.field_border);
    }

    let fullest = counts.iter().copied().max().unwrap_or(0).max(1) as f32;
    let bar_width = inner.width() / counts.len().max(1) as f32;
    for (index, count) in counts.iter().enumerate() {
        let bar_height = inner.height() * *count as f32 / fullest;
        let x = inner.left() + index as f32 * bar_width;
        painter.rect_filled(Rect::from_min_max(pos2(x, inner.bottom() - bar_height), pos2(x + bar_width, inner.bottom())), 0.0, tokens.text_dim);
    }

    let x_at = |handle: LevelsHandle| track_x(&inner.x_range(), handle.position(levels));
    let (low, high) = (x_at(LevelsHandle::InputLow), x_at(LevelsHandle::InputHigh));
    let shade = tokens.field.gamma_multiply(OUTSIDE_RANGE_SHADE);
    painter.rect_filled(Rect::from_x_y_ranges(inner.left()..=low, inner.y_range()), 0.0, shade);
    painter.rect_filled(Rect::from_x_y_ranges(high..=inner.right(), inner.y_range()), 0.0, shade);
    vertical_line(low, tokens.accent);
    vertical_line(high, tokens.accent);
    vertical_line(x_at(LevelsHandle::Gamma), tokens.text_dim.gamma_multiply(GAMMA_LINE_OPACITY));
}

/// A black-to-white strip: the output values the output handles pick from
fn paint_output_gradient(ui: &Ui, strip: Rect) {
    let mut mesh = egui::Mesh::default();
    for (corner, color) in
        [(strip.left_top(), Color32::BLACK), (strip.right_top(), Color32::WHITE), (strip.right_bottom(), Color32::WHITE), (strip.left_bottom(), Color32::BLACK)]
    {
        mesh.colored_vertex(corner, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);

    let painter = ui.painter();
    painter.add(mesh);
    painter.rect_stroke(strip, 0.0, Stroke::new(1.0, Tokens::get(ui.ctx()).field_border), egui::StrokeKind::Outside);
}

/// Each handle filled with its gray and its value beside it, the active one first and overlapping values left out
fn paint_handles_and_values(ui: &Ui, rect: Rect, responses: &[(LevelsHandle, Response)], levels: &Levels, typing: Option<LevelsHandle>) {
    let track = track(rect);
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let font = egui::FontId::proportional(VALUE_FONT_SIZE);
    let is_active = |response: &Response| response.dragged() || response.hovered();
    let mut printed: Vec<Rect> = Vec::new();
    let mut by_priority: Vec<&(LevelsHandle, Response)> = responses.iter().collect();
    by_priority.sort_by_key(|(_, response)| !is_active(response));

    for &(handle, ref response) in by_priority {
        let x = track_x(&track, handle.position(levels));
        let fill = match handle {
            LevelsHandle::InputLow => Color32::BLACK,
            LevelsHandle::Gamma => Color32::from_gray(GAMMA_HANDLE_GRAY),
            LevelsHandle::InputHigh => Color32::WHITE,
            LevelsHandle::OutputLow | LevelsHandle::OutputHigh => Color32::from_gray((handle.value(levels).clamp(0.0, 1.0) * 255.0).round() as u8),
        };
        let outline = if response.dragged() {
            tokens.accent
        } else if response.hovered() {
            tokens.text_strong
        } else {
            tokens.text
        };
        let points_down = LevelsHandle::INPUT.contains(&handle);
        painter.add(egui::Shape::convex_polygon(handle_outline(x, handle_top(rect, handle), points_down), fill, Stroke::new(1.0, outline)));
        if typing == Some(handle) {
            continue;
        }

        let color = if is_active(response) { tokens.text_strong } else { tokens.text_dim };
        let galley = painter.layout_no_wrap(shown_value(handle.value(levels)), font.clone(), color);
        let text_rect = value_rect(rect, handle, levels, galley.size());
        if printed.iter().any(|other| other.expand(VALUE_SPACING).intersects(text_rect)) {
            continue;
        }

        painter.galley(text_rect.min, galley, color);
        printed.push(text_rect);
    }
}

/// A pentagon pointer centred on `x`: flat side away from the track, tip touching it
fn handle_outline(x: f32, top: f32, points_down: bool) -> Vec<Pos2> {
    let bottom = top + HANDLE_HEIGHT;
    let (flat, side_end, tip) = if points_down { (top, top + HANDLE_SIDE_HEIGHT, bottom) } else { (bottom, bottom - HANDLE_SIDE_HEIGHT, top) };
    vec![
        pos2(x - HANDLE_HALF_WIDTH, flat),
        pos2(x + HANDLE_HALF_WIDTH, flat),
        pos2(x + HANDLE_HALF_WIDTH, side_end),
        pos2(x, tip),
        pos2(x - HANDLE_HALF_WIDTH, side_end),
    ]
}

/// Opens the box that types `handle`'s value, holding its current value
fn start_typing(ui: &Ui, typing_id: Id, handle: LevelsHandle, levels: &Levels) {
    let text = format!("{:.VALUE_DECIMALS$}", handle.value(levels));
    ui.data_mut(|data| data.insert_temp(typing_id, (handle, text)));
}

/// The floating box a value is typed into: Enter or a click elsewhere applies it as one undo step, Escape drops it
fn typed_value(ui: &Ui, rect: Rect, typing_id: Id, (handle, mut text): (LevelsHandle, String), opened: bool, levels: &mut Levels) -> Option<bool> {
    let field_id = typing_id.with("field");
    let field = value_rect(rect, handle, levels, vec2(TYPED_VALUE_WIDTH, VALUE_ROW_HEIGHT));
    // Floating over the editor, so opening it never moves the rest of the panel
    let response = egui::Area::new(typing_id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(field.min)
        .show(ui.ctx(), |ui| {
            let text_edit = egui::TextEdit::singleline(&mut text)
                .id(field_id)
                .font(egui::FontId::proportional(VALUE_FONT_SIZE))
                .align(egui::Align2::CENTER_CENTER)
                .margin(TYPED_VALUE_MARGIN);
            ui.add_sized(field.size(), text_edit)
        })
        .inner;

    // The app can consume Escape before the box sees it, so read the raw event
    let escape = ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));

    if opened {
        // Typing replaces the value; selected only now, as the unfocused box collapses a selection
        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), field_id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&text)));
        state.store(ui.ctx(), field_id);
        response.request_focus();
        return None;
    }

    if response.has_focus() && !escape {
        ui.data_mut(|data| data.insert_temp(typing_id, (handle, text)));
        return None;
    }

    ui.data_mut(|data| data.remove::<(LevelsHandle, String)>(typing_id));
    ui.memory_mut(|memory| memory.surrender_focus(field_id));
    // A box left open while the editor was hidden is dropped, not applied later
    if !response.lost_focus() {
        return None;
    }

    let typed = text.trim().parse::<f32>().ok().filter(|value| value.is_finite());
    let (false, Some(typed)) = (escape, typed) else { return None };

    let mut next = *levels;
    handle.set_value(&mut next, typed);
    if next == *levels {
        return None;
    }

    *levels = next;
    Some(false)
}
