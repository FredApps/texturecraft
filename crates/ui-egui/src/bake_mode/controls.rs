use crate::i18n::tr;
use crate::theme::{Tokens, caption};
use egui::collapsing_header::CollapsingState;
use egui::{RichText, Ui};

pub(super) fn section_header(ui: &mut Ui, title: &str, mut is_enabled: Option<&mut bool>, collapsing_state: Option<&mut CollapsingState>) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let text_color = if is_enabled.as_deref().copied().unwrap_or(true) { tokens.text_strong } else { tokens.text_dim };
    let mut checkbox_rect = None;
    let mut arrow_rect = None;

    let header = egui::Frame::NONE.fill(tokens.row).corner_radius(tokens.cr()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_min_height(ui.spacing().interact_size.y);
            let checkbox = is_enabled.as_deref_mut().map(|is_enabled| ui.checkbox(is_enabled, ""));
            let title_response = ui.add(egui::Label::new(caption(title).color(text_color)).selectable(false));
            if let Some(checkbox) = checkbox {
                checkbox_rect = Some(checkbox.rect);
                checkbox.labelled_by(title_response.id);
            }

            if collapsing_state.is_some() {
                arrow_rect = Some(
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.allocate_exact_size(egui::vec2(ui.spacing().indent, ui.spacing().icon_width), egui::Sense::hover()).0
                    })
                    .inner,
                );
            }
        });
    });

    if let (Some(collapsing_state), Some(arrow_rect)) = (collapsing_state, arrow_rect) {
        let mut toggle_rect = header.response.rect;
        if let Some(checkbox_rect) = checkbox_rect {
            let checkbox_margin = ui.spacing().item_spacing.x * 0.5;
            if ui.style().right_to_left {
                toggle_rect.max.x = checkbox_rect.left() - checkbox_margin;
            } else {
                toggle_rect.min.x = checkbox_rect.right() + checkbox_margin;
            }
        }

        let response = ui.interact(toggle_rect, ui.make_persistent_id("section_header"), egui::Sense::click());
        if response.clicked() {
            collapsing_state.toggle(ui);
        }

        let chevron = crate::widgets::section_chevron(ui, collapsing_state.is_open());
        ui.put(arrow_rect, egui::Label::new(chevron).selectable(false));
    }

    is_enabled.as_deref().copied().unwrap_or(true)
}

pub(super) fn section(ui: &mut Ui, title: &str, is_enabled: Option<&mut bool>, contents: impl FnOnce(&mut Ui)) -> bool {
    let mut collapsing_state = CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id("section"), true);
    if !section_header(ui, title, is_enabled, Some(&mut collapsing_state)) {
        collapsing_state.store(ui.ctx());
        return false;
    }

    collapsing_state
        .show_body_unindented(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    contents(ui);
                    ui.add_space(6.0);
                    ui.button(tr("Restore Defaults")).clicked()
                })
                .inner
        })
        .is_some_and(|response| response.inner)
}

pub(super) fn choice<T: Copy + PartialEq>(ui: &mut Ui, label: &str, value: &mut T, options: &[(T, &str)]) {
    let caption = ui.label(label);
    let selected = options.iter().find(|(option, _)| option == value).map_or("", |(_, label)| *label);
    egui::ComboBox::from_id_salt(label)
        .width(ui.available_width())
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (option, label) in options {
                ui.selectable_value(value, *option, *label);
            }
        })
        .response
        .labelled_by(caption.id);
}

pub(super) fn number(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, decimals: usize) {
    let response = crate::widgets::slider(ui, label, value, range.clone(), decimals);
    if response.changed() && (response.dragged() || response.clicked()) {
        let scale = 10_f32.powi(decimals as i32);
        *value = ((*value * scale).round() / scale).clamp(*range.start(), *range.end());
    }
}

pub(super) fn integer(ui: &mut Ui, label: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>) {
    let mut number_value = *value as f32;
    let response = crate::widgets::slider(ui, label, &mut number_value, *range.start() as f32..=*range.end() as f32, 0);
    if response.changed() {
        *value = (number_value.round() as u32).clamp(*range.start(), *range.end());
    }
}

pub(super) fn text_input(ui: &mut Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()));
}

pub(super) fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(Tokens::get(ui.ctx()).text_dim).size(11.0));
}
