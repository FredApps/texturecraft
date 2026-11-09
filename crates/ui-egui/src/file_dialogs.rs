//! Native file dialogs that never block the frame (craft-dialogs).
//!
//! An action that needs a path calls [`TextureCraftApp::ask_file`] and returns. The dialog shows at
//! the end of the frame, and the continuation runs with the answer on a later frame. The browser
//! build has no native dialogs (`web_files.rs`).

use std::panic::Location;
use std::path::PathBuf;
use std::sync::Arc;

pub use craft_dialogs::{Answer, FileDialog, Override, Request};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};

/// Whether an answer still needs the project that was open when the dialog was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    /// The continuation reads or changes the open project, and does not run on another one.
    Project,
    /// The continuation does not depend on the open project.
    App,
}

/// What runs with the answer. The list is empty when nothing was chosen.
type Then = Box<dyn FnOnce(&mut TextureCraftApp, Vec<PathBuf>)>;

/// Which dialog was asked for, for which project and by which code. Asking again with all three
/// the same brings the open panel to the front instead of queuing a second one.
#[derive(Clone, Debug, PartialEq)]
struct AskKey {
    request: Request,
    project: Option<u64>,
    origin: &'static Location<'static>,
}

/// A dialog's continuation.
pub struct Asked {
    key: AskKey,
    then: Then,
}

/// The app's dialogs and the requests not yet answered.
pub struct FileDialogs {
    dialogs: craft_dialogs::Dialogs<Asked>,
    asked: Vec<AskKey>,
}

impl FileDialogs {
    /// The dialogs of the app whose context is `ctx`: an answer wakes its event loop.
    pub fn new(ctx: egui::Context) -> FileDialogs {
        FileDialogs { dialogs: craft_dialogs::Dialogs::new(Arc::new(move || ctx.request_repaint())), asked: Vec::new() }
    }

    /// Answers every request with `answer_with` instead of a native dialog (tests).
    pub fn set_override(&mut self, answer_with: Option<Override>) {
        self.dialogs.set_override(answer_with);
    }

    /// Requests that have not been answered yet.
    pub fn pending(&self) -> usize {
        self.asked.len()
    }

    /// Shows the next dialog asked for. Call at the end of every frame with the window.
    pub fn show_pending(&mut self, frame: &eframe::Frame) {
        self.dialogs.show_pending(Some(frame));
    }
}

impl TextureCraftApp {
    /// Queues `dialog`. `then` runs on a later frame with the chosen paths, or with none after
    /// Cancel, after a dialog that could not be shown, and when `bound` is [`Bound::Project`] and
    /// another project has been opened since. Returns false, and drops `then`, when the same dialog
    /// is already showing or waiting.
    #[track_caller]
    pub(crate) fn ask_file(&mut self, dialog: FileDialog, bound: Bound, then: impl FnOnce(&mut TextureCraftApp, Vec<PathBuf>) + 'static) -> bool {
        let project = (bound == Bound::Project).then_some(self.session.project_id);
        let key = AskKey { request: Request::File(dialog.clone()), project, origin: Location::caller() };
        if self.file_dialogs.asked.contains(&key) {
            self.file_dialogs.dialogs.raise();
            return false;
        }
        self.file_dialogs.asked.push(key.clone());
        self.file_dialogs.dialogs.ask(dialog, Asked { key, then: Box::new(then) });
        true
    }

    /// Runs the continuations of the dialogs that have answered.
    pub(crate) fn run_file_dialog_answers(&mut self) {
        for (asked, answer) in self.file_dialogs.dialogs.take_answers() {
            let Asked { key, then } = asked;
            if let Some(i) = self.file_dialogs.asked.iter().position(|k| *k == key) {
                self.file_dialogs.asked.remove(i);
            }
            let paths = match answer {
                Answer::Paths(paths) => paths,
                Answer::Unavailable(why) => {
                    self.toast(trf("The file dialog couldn't be shown: {error}", &[("error", &why)]), true);
                    Vec::new()
                }
                Answer::Cancelled | Answer::Message(_) => Vec::new(),
            };
            let moved_on = key.project.is_some_and(|project| project != self.session.project_id);
            if moved_on && !paths.is_empty() {
                self.toast(tr("The project changed while the dialog was open, so nothing was done"), true);
            }
            then(self, if moved_on { Vec::new() } else { paths });
        }
    }
}
