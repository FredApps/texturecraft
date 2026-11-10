//! The filter slot atop a filter's properties: thumbnail and name, opening a picker that swaps it

use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::{Filter, LayerId};
use texturecraft_engine::filter_lib::{filter_effects, find};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;
use crate::thumbs::Thumbs;
use crate::widgets::elided;

const SLOT_HEIGHT: f32 = 52.0;
const SLOT_ROUNDING: f32 = 3.0;
const BORDER_WIDTH: f32 = 1.0;
const THUMBNAIL_SIZE: f32 = 40.0;
const THUMBNAIL_INSET: f32 = 6.0;
const CAPTION_FONT_SIZE: f32 = 11.0;
const NAME_FONT_SIZE: f32 = 13.0;
/// Gap between the caption's and the name's centers
const LINE_GAP: f32 = 15.0;
const CARET_INSET: f32 = 14.0;
const CARET_FONT_SIZE: f32 = 14.0;
/// Gap between the slot and its keyboard focus outline
const FOCUS_MARGIN: f32 = 2.0;

/// The slot of filter effect `effect`: a pick runs `effect.set {filter}`, masks offer only mask filters
pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui, effect: u64, filter: &Filter) {
    let ctx = ui.ctx().clone();
    let on_mask = is_on_mask(app, effect);
    let current = find(filter.label()).map(|def| def.id);
    let thumbnail = current.and_then(|id| app.thumbs.filter(&ctx, id));
    let name = tr(filter.label());

    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), SLOT_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));

    let popup = ui.make_persistent_id(("filter_picker", effect));
    let open_requested = take_open_request(&ctx, effect);

    if response.clicked() || (open_requested && !egui::Popup::is_id_open(&ctx, popup)) {
        resource_picker::toggle(&ctx, popup);
    }

    let highlighted = response.hovered() || egui::Popup::is_id_open(&ctx, popup);
    paint(ui, rect, highlighted, response.has_focus(), thumbnail.as_ref(), name);

    let items = |thumbs: &mut Thumbs| {
        filter_effects()
            .iter()
            .filter(|def| def.mask || !on_mask)
            .map(|def| ResourcePickerItem { id: def.id, label: tr(def.label), group: tr(def.group), thumbnail: thumbs.filter(&ctx, def.id) })
            .collect()
    };
    let picked = resource_picker::show(&response, popup, &mut app.thumbs, current, items, ShelfPreview::Filter);
    if let Some(id) = picked.filter(|id| Some(*id) != current) {
        app.run("effect.set", json!({"effect": effect, "filter": id}));
    }
}

/// Has the slot of filter effect `effect` open its picker if it shows this frame or the next, as Add filter does for the filter it adds
pub(crate) fn open_picker_when_shown(ctx: &egui::Context, effect: u64) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(open_request_id(), (effect, frame)));
}

/// Whether an open request for `effect`'s slot is waiting, removing it; a request older than a frame lapses
fn take_open_request(ctx: &egui::Context, effect: u64) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let request = ctx.data_mut(|data| data.remove_temp::<(u64, u64)>(open_request_id()));

    // Properties can draw before or after the menu that asked, so a request lives for this frame and the next
    request.is_some_and(|(requested_effect, requested_frame)| requested_effect == effect && frame <= requested_frame + 1)
}

/// Where the effect whose slot should open its picker waits
fn open_request_id() -> egui::Id {
    egui::Id::unique("filter_slot_open_request")
}

/// Whether filter effect `effect` sits in a mask rather than the layer content
fn is_on_mask(app: &TextureCraftApp, effect: u64) -> bool {
    let project = &app.session.doc.project;
    let owner = project.effect(LayerId(effect)).and_then(|(owner, _)| project.layers.get(&owner));
    owner.and_then(|layer| layer.mask.as_ref()).is_some_and(|mask| mask.effects.iter().any(|masked| masked.id.0 == effect))
}

/// The slot's box, highlighted while hovered or its picker is open and outlined while focused: thumbnail, "Filter" over the name, caret
fn paint(ui: &Ui, rect: Rect, highlighted: bool, focused: bool, thumbnail: Option<&egui::TextureHandle>, name: &str) {
    let tk = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let (fill, border) = if highlighted { (tk.row_hover, tk.button_border) } else { (tk.field, tk.field_border) };
    painter.rect(rect, SLOT_ROUNDING, fill, Stroke::new(BORDER_WIDTH, border), egui::StrokeKind::Inside);
    if focused {
        painter.rect_stroke(rect.expand(FOCUS_MARGIN), SLOT_ROUNDING, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
    }

    if let Some(texture) = thumbnail {
        let image = Rect::from_min_size(pos2(rect.left() + THUMBNAIL_INSET, rect.center().y - THUMBNAIL_SIZE * 0.5), vec2(THUMBNAIL_SIZE, THUMBNAIL_SIZE));
        painter.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }

    // Long names are cut short rather than run under the thumbnail or the caret
    let text_left = rect.left() + THUMBNAIL_INSET * 2.0 + THUMBNAIL_SIZE;
    let text_right = rect.right() - CARET_INSET * 2.0;
    let text_center_x = (text_left + text_right) * 0.5;
    let lines = [(tr("Filter"), CAPTION_FONT_SIZE, tk.text_dim, -LINE_GAP * 0.5), (name, NAME_FONT_SIZE, tk.text_strong, LINE_GAP * 0.5)];
    for (text, size, color, offset) in lines {
        let galley = elided(painter, text, size, color, text_right - text_left);
        painter.galley(pos2(text_center_x, rect.center().y + offset) - galley.size() * 0.5, galley, color);
    }

    let caret_color = if highlighted { tk.text } else { tk.text_dim };
    painter.text(pos2(rect.right() - CARET_INSET, rect.center().y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(CARET_FONT_SIZE), caret_color);
}
