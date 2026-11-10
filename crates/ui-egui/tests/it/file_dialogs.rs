//! Native file dialogs answer on a later frame; the tests answer them without a window.

use std::path::{Path, PathBuf};

use egui::{ViewportCommand, ViewportEvent, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::Session;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::{Answer, Request};
use texturecraft_ui_egui::state::Unsaved;

fn harness(modified: bool) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.modified = modified;
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn answer(h: &mut Harness<'_, TextureCraftApp>, answer_with: impl FnMut(&Request) -> Answer + 'static) {
    h.state_mut().file_dialogs.set_override(Some(Box::new(answer_with)));
}

fn invoke(h: &mut Harness<'_, TextureCraftApp>, id: &str) {
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, id);
}

/// A path in a new temporary folder.
fn temp_file(test: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-file-dialogs-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn project_named(name: &str, file: &PathBuf) {
    let session = Session::new_project(name, texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    std::fs::write(file, session.to_project_bytes().unwrap()).unwrap();
}

#[test]
fn open_returns_at_once_and_the_answer_opens_the_project_later() {
    let file = temp_file("open", "picked.tcraft");
    project_named("Picked", &file);
    let mut h = harness(false);
    let path = file.clone();
    answer(&mut h, move |_| Answer::Paths(vec![path.clone()]));
    invoke(&mut h, "file.open");
    assert_ne!(h.state().session.doc.project.name, "Picked", "the menu click does not wait for the dialog");
    h.run_steps(3);
    assert_eq!(h.state().session.doc.project.name, "Picked");
    assert_eq!(h.state().ui.recent.first().map(String::as_str), Some(file.display().to_string().as_str()));
}

#[test]
fn a_canceled_dialog_changes_nothing() {
    let mut h = harness(false);
    let name = h.state().session.doc.project.name.clone();
    answer(&mut h, |_| Answer::Cancelled);
    invoke(&mut h, "file.open");
    h.run_steps(3);
    assert_eq!(h.state().session.doc.project.name, name);
    assert!(h.state().toasts.is_empty());
    assert_eq!(h.state().file_dialogs.pending(), 0);
}

#[test]
fn a_dialog_that_cannot_show_is_reported() {
    let mut h = harness(false);
    answer(&mut h, |_| Answer::Unavailable("no portal".into()));
    invoke(&mut h, "file.open");
    h.run_steps(3);
    assert!(h.state().toasts.iter().any(|t| t.error && t.text.contains("no portal")));
}

#[test]
fn choosing_the_same_menu_item_twice_asks_once() {
    let mut h = harness(false);
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = asked.clone();
    answer(&mut h, move |_| {
        count.set(count.get() + 1);
        Answer::Cancelled
    });
    invoke(&mut h, "file.open");
    invoke(&mut h, "file.open");
    assert_eq!(h.state().file_dialogs.pending(), 1);
    h.run_steps(3);
    assert_eq!(asked.get(), 1);
}

#[test]
fn save_as_and_save_a_copy_are_separate_requests() {
    let mut h = harness(false);
    answer(&mut h, |_| Answer::Cancelled);
    invoke(&mut h, "file.saveAs");
    invoke(&mut h, "file.saveCopy");
    assert_eq!(h.state().file_dialogs.pending(), 2, "the second one does not take the first one's place");
}

#[test]
fn save_as_offers_the_project_format_and_a_name() {
    let mut h = harness(false);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let store = seen.clone();
    answer(&mut h, move |request| {
        *store.borrow_mut() = Some(request.clone());
        Answer::Cancelled
    });
    invoke(&mut h, "file.saveAs");
    h.run_steps(3);
    let Some(Request::File(dialog)) = seen.borrow().clone() else { panic!("no file dialog was asked for") };
    assert_eq!(dialog.filters.first().map(|f| f.extensions.clone()), Some(vec![texturecraft_engine::core::PROJECT_EXT.to_string()]));
    assert!(dialog.file_name.is_some_and(|name| name.ends_with(texturecraft_engine::core::PROJECT_EXT)));
}

#[test]
fn a_save_answer_for_a_replaced_project_saves_nothing() {
    let file = temp_file("replaced", "kept.tcraft");
    let mut h = harness(true);
    let path = file.clone();
    answer(&mut h, move |_| Answer::Paths(vec![path.clone()]));
    invoke(&mut h, "file.saveAs");
    h.state_mut().session.project_id += 1;
    h.run_steps(3);
    assert!(!file.exists(), "the answer belonged to the project that was open when it was asked");
    assert!(h.state().toasts.iter().any(|t| t.error));
    assert!(h.state().session.modified);
}

const PROJECT_CHANGED: &str = "The project changed while the dialog was open";

/// A project-bound dialog asked by `ask`, answered after another project was opened.
fn answered_after_the_project_changed(ask: impl FnOnce(&mut Harness<'_, TextureCraftApp>), file: &Path) -> Harness<'static, TextureCraftApp> {
    let mut h = harness(false);
    h.state_mut().session.doc.project.mesh_path.clear();
    let path = file.to_path_buf();
    answer(&mut h, move |_| Answer::Paths(vec![path.clone()]));
    ask(&mut h);
    h.state_mut().session.project_id += 1;
    h.run_steps(3);
    h
}

#[test]
fn every_project_dialog_does_nothing_for_a_replaced_project() {
    let file = temp_file("replaced-actions", "chosen.obj");
    for id in ["material.load", "resource.import", "file.importMaterialX", "file.exportMesh", "file.reimportMesh"] {
        let h = answered_after_the_project_changed(|h| invoke(h, id), &file);
        assert!(h.state().toasts.iter().any(|t| t.error && t.text.starts_with(PROJECT_CHANGED)), "{id}");
        assert!(!file.exists(), "{id} wrote nothing");
    }
    let bake = answered_after_the_project_changed(
        |h| {
            let ctx = h.ctx.clone();
            texturecraft_ui_egui::web_files::import_bake_mesh(h.state_mut(), &ctx, false);
        },
        &file,
    );
    assert!(bake.state().toasts.iter().any(|t| t.error && t.text.starts_with(PROJECT_CHANGED)), "bake meshes");
    let hdr = answered_after_the_project_changed(
        |h| {
            let ctx = h.ctx.clone();
            texturecraft_ui_egui::web_files::import_environment(h.state_mut(), &ctx);
        },
        &file,
    );
    assert!(hdr.state().toasts.iter().any(|t| t.error && t.text.starts_with(PROJECT_CHANGED)), "Import HDR");
}

#[test]
fn reimport_reads_the_remembered_file_without_asking() {
    let file = temp_file("reimport", "mesh.obj");
    std::fs::write(&file, "").unwrap();
    let mut h = harness(false);
    h.state_mut().session.doc.project.mesh_path = file.display().to_string();
    answer(&mut h, |_| Answer::Cancelled);
    invoke(&mut h, "file.reimportMesh");
    assert_eq!(h.state().file_dialogs.pending(), 0, "the file is still there, so no dialog was asked for");
}

#[test]
fn a_mesh_change_keeps_the_answer_for_the_same_project() {
    let file = temp_file("mesh-change", "kept.tcraft");
    let mut h = harness(true);
    let path = file.clone();
    answer(&mut h, move |_| Answer::Paths(vec![path.clone()]));
    invoke(&mut h, "file.saveAs");
    h.state_mut().session.mesh_id += 1;
    h.run_steps(3);
    assert!(file.exists(), "a reimport or a UV layout change is still the same project");
}

fn request_close(h: &mut Harness<'_, TextureCraftApp>) {
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
    h.step();
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.clear();
    h.run_steps(2);
}

/// A root viewport command sent during `frames` steps.
fn sends(h: &mut Harness<'_, TextureCraftApp>, frames: usize, cmd: &ViewportCommand) -> bool {
    (0..frames).any(|_| {
        h.step();
        h.output().viewport_output.get(&ViewportId::ROOT).is_some_and(|v| v.commands.contains(cmd))
    })
}

fn untitled_with_prompt(h: &mut Harness<'_, TextureCraftApp>) {
    h.state_mut().session.path = None;
    request_close(h);
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the dialog is open");
}

#[test]
fn closing_during_a_stroke_finishes_it_and_asks() {
    let mut h = harness(false);
    h.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();
    assert!(!h.state().session.modified, "the stroke is not finished yet");
    request_close(&mut h);
    assert!(!h.state().session.stroke_active());
    assert!(h.state().session.modified);
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));
}

#[test]
fn save_in_the_prompt_goes_on_once_the_dialog_has_saved() {
    let file = temp_file("prompt-save", "saved.tcraft");
    let mut h = harness(true);
    untitled_with_prompt(&mut h);
    let path = file.clone();
    answer(&mut h, move |_| Answer::Paths(vec![path.clone()]));
    h.get_by_label("Save").click();
    assert!(sends(&mut h, 5, &ViewportCommand::Close), "the close goes on after the save");
    assert!(file.exists());
    assert!(!h.state().session.modified);
    assert!(h.state().ui.windows.unsaved.is_none());
}

#[test]
fn canceling_the_save_dialog_keeps_the_prompt_and_its_buttons() {
    let mut h = harness(true);
    untitled_with_prompt(&mut h);
    answer(&mut h, |_| Answer::Cancelled);
    h.get_by_label("Save").click();
    assert!(!sends(&mut h, 5, &ViewportCommand::Close));
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the question is still open");
    assert!(h.state().session.modified);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().ui.windows.unsaved.is_none(), "Cancel answers again after the Save dialog closed");
}

#[test]
fn a_save_elsewhere_does_not_answer_the_prompt() {
    let file = temp_file("prompt-other", "other.tcraft");
    let mut h = harness(true);
    untitled_with_prompt(&mut h);
    let path = file.display().to_string();
    h.state_mut().run("file.saveAs", serde_json::json!({ "path": path }));
    assert!(!h.state().session.modified);
    assert!(!sends(&mut h, 3, &ViewportCommand::Close), "only the prompt's own Save goes on");
    assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));
}
