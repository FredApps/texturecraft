//! A popup under a slot that picks a resource, such as a filter, from grouped shelf tiles searched by name

use egui::{Id, Response, Stroke, Ui};

use crate::i18n::tr;
use crate::resource_picker_item::ResourcePickerItem;
use crate::shelf;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;
use crate::thumbs::Thumbs;

/// Four shelf tiles and the scrollbar
const PICKER_WIDTH: f32 = 320.0;
/// The grid's height, fixed so the popup keeps its place while searching
const PICKER_HEIGHT: f32 = 340.0;
const CURRENT_OUTLINE_WIDTH: f32 = 1.5;
const TILE_ROUNDING: f32 = 3.0;

/// Opens the picker `popup_id` with an empty, focused search field, or closes it
pub(crate) fn toggle(ctx: &egui::Context, popup_id: Id) {
    if egui::Popup::is_id_open(ctx, popup_id) {
        egui::Popup::close_id(ctx, popup_id);
        return;
    }

    egui::Popup::open_id(ctx, popup_id);
    ctx.data_mut(|data| {
        data.insert_temp(search_id(popup_id), String::new());
        data.insert_temp(focus_id(popup_id), true);
    });
}

/// Draws picker `popup_id` at `slot` while open and returns the id clicked (`items` runs only then)
pub(crate) fn show(
    slot: &Response,
    popup_id: Id,
    thumbs: &mut Thumbs,
    current: Option<&str>,
    items: impl FnOnce(&mut Thumbs) -> Vec<ResourcePickerItem>,
    preview: impl Fn(&'static str) -> ShelfPreview,
) -> Option<&'static str> {
    let ctx = slot.ctx.clone();
    if !egui::Popup::is_id_open(&ctx, popup_id) {
        return None;
    }

    let items = items(thumbs);
    let mut picked = None;
    egui::Popup::from_response(slot)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::BOTTOM_END)
        .width(PICKER_WIDTH)
        .show(|ui| {
            ui.set_width(PICKER_WIDTH);
            let just_opened = ui.data_mut(|data| data.remove_temp::<bool>(focus_id(popup_id))).unwrap_or(false);
            let query = search_field(ui, popup_id, just_opened);
            egui::ScrollArea::vertical().max_height(PICKER_HEIGHT).show(ui, |ui| {
                // A search that leaves few tiles must not shrink the popup, or egui moves it
                ui.set_min_height(PICKER_HEIGHT);
                picked = grid(ui, thumbs, &items, &query, current, just_opened, &preview);
            });
        });

    if picked.is_some() {
        egui::Popup::close_id(&ctx, popup_id);
    }
    picked
}

/// The search field, focused the frame the picker opens; returns the lowercase query
fn search_field(ui: &mut Ui, popup_id: Id, just_opened: bool) -> String {
    let mut search: String = ui.data(|data| data.get_temp(search_id(popup_id))).unwrap_or_default();
    let field = ui.add(egui::TextEdit::singleline(&mut search).hint_text(tr("Search")).desired_width(f32::INFINITY));
    if just_opened {
        field.request_focus();
    }

    let query = search.trim().to_lowercase();
    ui.data_mut(|data| data.insert_temp(search_id(popup_id), search));
    query
}

/// The items matching `query` as tiles under group headers, `current` outlined; returns the one clicked
fn grid(
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    items: &[ResourcePickerItem],
    query: &str,
    current: Option<&str>,
    reveal_current: bool,
    preview: &impl Fn(&'static str) -> ShelfPreview,
) -> Option<&'static str> {
    let tk = Tokens::get(ui.ctx());
    let shown: Vec<&ResourcePickerItem> = items.iter().filter(|item| shelf::matches(query, &[item.label, item.id])).collect();
    let mut picked = None;
    for group in shelf::categories(&shown, |item| item.group) {
        shelf::header(ui, group);
        shelf::tiles(ui, |ui| {
            for item in shown.iter().filter(|item| item.group == group) {
                let response = shelf::tile(ui, thumbs, item.thumbnail.as_ref(), || preview(item.id), item.label, None);
                if current == Some(item.id) {
                    if reveal_current {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    ui.painter().rect_stroke(response.rect, TILE_ROUNDING, Stroke::new(CURRENT_OUTLINE_WIDTH, tk.accent), egui::StrokeKind::Inside);
                }
                if response.clicked() {
                    picked = Some(item.id);
                }
            }
        });
    }
    picked
}

/// Where the picker keeps its search text
fn search_id(popup_id: Id) -> Id {
    popup_id.with("search")
}

/// Set while the picker waits for its first frame open
fn focus_id(popup_id: Id) -> Id {
    popup_id.with("focus")
}
