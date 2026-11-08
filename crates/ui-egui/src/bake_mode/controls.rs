use crate::i18n::tr;
use crate::theme::{Tokens, caption};
use egui::collapsing_header::CollapsingState;
use egui::{RichText, Ui};

/// A section's title bar with its chevron; clicking it opens or closes the section
fn section_header(ui: &mut Ui, title: &str, collapsing_state: &mut CollapsingState) {
    let tokens = Tokens::get(ui.ctx());
    let header = egui::Frame::NONE.fill(tokens.row).corner_radius(tokens.cr()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_min_height(ui.spacing().interact_size.y);
            ui.add(egui::Label::new(caption(title).color(tokens.text_strong)).selectable(false));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.allocate_exact_size(egui::vec2(ui.spacing().indent, ui.spacing().icon_width), egui::Sense::hover()).0
            })
            .inner
        })
        .inner
    });

    let response = ui.interact(header.response.rect, ui.make_persistent_id("section_header"), egui::Sense::click());
    if response.clicked() {
        collapsing_state.toggle(ui);
    }

    let chevron = crate::widgets::section_chevron(ui, collapsing_state.is_open());
    ui.put(header.inner, egui::Label::new(chevron).selectable(false));
}

/// A collapsible section with `contents` and Restore Defaults; returns whether that was clicked
pub(super) fn section(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui)) -> bool {
    let mut collapsing_state = CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id("section"), true);
    section_header(ui, title, &mut collapsing_state);

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
