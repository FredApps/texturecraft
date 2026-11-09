//! Closing the app, or opening or closing a project, with unsaved changes asks whether to save
//! them first.

use egui::{ViewportCommand, ViewportEvent, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Unsaved;

fn harness(modified: bool) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.modified = modified;
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

/// The window's close button, for one frame.
fn request_close(h: &mut Harness<'_, TextureCraftApp>) {
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
    h.step();
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.clear();
}

/// The viewport commands of the last frame.
fn sent(h: &Harness<'_, TextureCraftApp>, cmd: &ViewportCommand) -> bool {
    h.output().viewport_output.get(&ViewportId::ROOT).is_some_and(|v| v.commands.contains(cmd))
}

#[test]
fn closing_with_changes_asks_and_dont_save_closes() {
    let mut h = harness(true);
    request_close(&mut h);
    assert!(sent(&h, &ViewportCommand::CancelClose), "the close waits for an answer");
    h.run_steps(2);
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the dialog is open");
    h.get_by_label("Don't save").click();
    h.step();
    assert!(sent(&h, &ViewportCommand::Close), "Don't save closes the app");
    assert!(h.state().ui.windows.unsaved.is_none());
}

#[test]
fn cancel_keeps_the_app_open() {
    let mut h = harness(true);
    request_close(&mut h);
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.step();
    assert!(!sent(&h, &ViewportCommand::Close));
    assert!(h.state().ui.windows.unsaved.is_none(), "Cancel closes the dialog");
    assert!(h.state().session.modified, "and keeps the changes");
}

#[test]
fn closing_without_changes_does_not_ask() {
    let mut h = harness(false);
    request_close(&mut h);
    // macOS keeps the window for `-[NSApplication terminate:]` to close; elsewhere it closes now.
    assert_eq!(sent(&h, &ViewportCommand::CancelClose), cfg!(target_os = "macos"));
    assert!(h.state().ui.windows.unsaved.is_none());
}

/// `ui` does not run while the window is minimized, so the close decision is made in `logic`.
#[test]
fn closing_a_minimized_window_with_changes_brings_it_back_to_ask() {
    let mut h = harness(true);
    let root = h.input_mut().viewports.entry(ViewportId::ROOT).or_default();
    root.minimized = Some(true);
    root.events.push(ViewportEvent::Close);
    h.step();
    assert!(sent(&h, &ViewportCommand::CancelClose), "the close waits for an answer");
    assert!(sent(&h, &ViewportCommand::Minimized(false)), "the window comes back");
    assert!(sent(&h, &ViewportCommand::Focus));
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));
}

/// A project picked or dropped in the browser arrives as bytes; it waits on the same dialog.
#[test]
fn a_dropped_project_waits_on_the_dialog() {
    let mut h = harness(true);
    let dropped = texturecraft_engine::Session::new_project("Dropped", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    texturecraft_ui_egui::web_files::load(h.state_mut(), "dropped.tcraft", dropped.to_project_bytes().unwrap());
    h.run_steps(4);
    assert!(matches!(h.state().ui.windows.unsaved, Some(Unsaved::Load(ref name, _)) if name == "dropped.tcraft"), "the dialog is open");
    assert_ne!(h.state().session.doc.project.name, "Dropped", "the project stays until answered");
    h.get_by_label("Don't save").click();
    h.run_steps(2);
    assert_eq!(h.state().session.doc.project.name, "Dropped", "Don't save opens it");
    assert!(h.state().ui.windows.unsaved.is_none());
}

#[test]
fn closing_the_project_with_changes_asks_first() {
    let mut h = harness(true);
    h.state_mut().session.doc.project.name = "Edited".into();
    let name = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.name.clone();
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "file.close");
    h.run_steps(4);
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Command("file.close".into())), "the dialog is open");
    assert_eq!(name(&h), "Edited", "the project stays until answered");
    h.get_by_label("Don't save").click();
    h.run_steps(2);
    assert_ne!(name(&h), "Edited", "Don't save closes the project");
    assert!(h.state().ui.windows.unsaved.is_none() && !h.state().session.modified);
}

/// The dialog fits one sentence and one button row instead of growing to the screen height
#[test]
fn the_dialog_fits_its_contents() {
    // The title bar, one line, one button row and margins take 119 px at 1x and the bug made it 420; the 31 px left over absorbs font and theme tweaks
    const MAX_HEIGHT: f32 = 150.0;

    let mut h = harness(true);
    request_close(&mut h);
    h.run_steps(2);

    let dialog = h.ctx.memory(|m| m.area_rect(egui::Id::unique("unsaved"))).expect("the dialog is shown");
    assert!(dialog.height() < MAX_HEIGHT, "the dialog is {} px tall", dialog.height());
}
