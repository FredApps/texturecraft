//! Keyboard shortcuts: every menu item and every command with a default shortcut, the user's
//! overrides (`UiState::keymap`, id → "Ctrl+Shift+E" or "" for unbound), conflict detection and
//! the shortcut editor window (Edit ▸ Settings ▸ Keyboard, Help ▸ Shortcuts list).

use std::collections::BTreeMap;

use craft_keymap::egui::Dispatcher;
use craft_keymap::{Active, Bindable, Chord, CommandRef, ConflictKind, KeyInput, Layout, Lookup, Mods, Overrides, Platform, Scope, Scopes};
use egui::{Event, Id, Key, KeyboardShortcut, Modifiers, RichText, vec2};
use egui_phosphor::regular as ic;

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::menus::CATALOG;
use crate::theme::Tokens;
use crate::widgets::{icon_button, section};

pub type Keymap = BTreeMap<String, String>;

/// Something a shortcut can trigger.
pub struct Binding {
    pub id: &'static str,
    /// Menu (or command group) it belongs to.
    pub group: &'static str,
    pub label: &'static str,
    pub default: Option<&'static str>,
}

/// Every bindable item: the menu catalog, then commands with a default shortcut.
pub fn bindings() -> &'static [Binding] {
    static ALL: std::sync::OnceLock<Vec<Binding>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut v: Vec<Binding> = Vec::new();
        for i in CATALOG.iter().filter(|i| i.label != "-") {
            if !v.iter().any(|b| b.id == i.id) {
                v.push(Binding { id: i.id, group: i.menu, label: i.label, default: i.shortcut });
            }
        }
        for c in texturecraft_engine::command_specs().iter().filter(|c| c.shortcut.is_some()) {
            if !v.iter().any(|b| b.id == c.id) {
                v.push(Binding { id: c.id, group: c.menu.first().copied().unwrap_or("Commands"), label: c.label, default: c.shortcut });
            }
        }
        v
    })
}

/// The shortcut `b` currently has.
pub fn shortcut<'a>(keymap: &'a Keymap, b: &'a Binding) -> Option<&'a str> {
    match keymap.get(b.id) {
        Some(s) => (!s.is_empty()).then_some(s.as_str()),
        None => b.default,
    }
}

/// The shortcut of item `id` (for menu labels and tooltips).
pub fn shortcut_of<'a>(keymap: &'a Keymap, id: &str) -> Option<&'a str> {
    bindings().iter().find(|b| b.id == id).and_then(|b| shortcut(keymap, b))
}

/// "Label (Shortcut)" for an icon button's tooltip.
pub fn tip(keymap: &Keymap, label: &str, id: &str) -> String {
    match shortcut_of(keymap, id) {
        Some(s) => format!("{label} ({s})"),
        None => label.to_owned(),
    }
}

/// craft-keymap's text of a shortcut: "Ctrl" here is the command modifier (Cmd on macOS), which the
/// crate writes "Cmd".
fn crate_text(text: &str) -> String {
    text.split('+').map(|part| if part == "Ctrl" { "Cmd" } else { part }).collect::<Vec<_>>().join("+")
}

/// Read "Ctrl+Shift+E" notation.
pub fn parse(text: &str) -> Option<Chord> {
    Chord::parse(&crate_text(text)).ok()
}

/// "Ctrl+Shift+E" notation (the inverse of [`parse`]).
pub fn format_shortcut(sc: &KeyboardShortcut) -> String {
    let m = sc.modifiers;
    let mut s = String::new();
    if m.command || m.ctrl || m.mac_cmd {
        s.push_str("Ctrl+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    s.push_str(sc.logical_key.name());
    s
}

/// Every bound shortcut as egui's shortcut type.
fn active(keymap: &Keymap) -> Vec<KeyboardShortcut> {
    bindings().iter().filter_map(|b| Some(parse(shortcut(keymap, b)?)?.to_egui_shortcut())).collect()
}

/// The catalog as craft-keymap sees it, in `bindings()` order: the first one listed wins a shared
/// shortcut.
fn catalog() -> Vec<Bindable> {
    bindings()
        .iter()
        .map(|b| {
            let item = Bindable::new(b.id, b.label, Scope::app());
            match b.default.and_then(parse) {
                Some(chord) => item.with_default(craft_keymap::Binding::new(chord)),
                None => item,
            }
        })
        .collect()
}

/// craft-keymap's text of a stored shortcut. A symbol that needs Shift is looked up without it, so
/// "Shift+Colon" is ":".
fn override_text(text: &str) -> String {
    match parse(text) {
        Some(chord) => Layout::new().resolve(&KeyInput::new(chord.mods).with_typed(chord.key)).unwrap_or(chord).to_string(),
        None => crate_text(text),
    }
}

/// The stored shortcuts as craft-keymap overrides ("" = no shortcut).
fn overrides(keymap: &Keymap) -> Overrides {
    let mut out = Overrides::new();
    for (id, text) in keymap {
        let texts = if text.is_empty() { Vec::new() } else { vec![override_text(text)] };
        // Only an overlong id or text is refused, and neither can be a shortcut.
        let _ = out.insert(id.clone(), texts);
    }
    out
}

/// The keymap with `keymap`'s shortcuts for `platform`.
fn resolved(keymap: &Keymap, platform: Platform) -> craft_keymap::Keymap {
    craft_keymap::Keymap::new(catalog(), Scopes::new(), platform).with_overrides(overrides(keymap))
}

/// The resolved keymap and the dispatcher that runs it against each frame's keys.
pub struct Bindings {
    stored: Keymap,
    map: craft_keymap::Keymap,
    dispatcher: Dispatcher,
}

impl Default for Bindings {
    fn default() -> Bindings {
        let stored = Keymap::new();
        let map = resolved(&stored, Platform::current());
        // Held keys repeat their command. Copy, Cut and Paste events count as Ctrl+C, X and V.
        Bindings { stored, map, dispatcher: Dispatcher::new().with_repeat(true).with_clipboard_events(true) }
    }
}

impl Bindings {
    /// Follows the stored shortcuts and the platform; a no-op while neither changed.
    pub(crate) fn sync(&mut self, keymap: &Keymap, platform: Platform) {
        if self.stored != *keymap || self.map.platform() != platform {
            self.stored = keymap.clone();
            self.map = resolved(keymap, platform);
        }
    }

    /// The ids of the commands this frame's key presses run, in press order. The presses are
    /// removed from the input.
    pub(crate) fn dispatch(&mut self, input: &mut egui::InputState) -> Vec<String> {
        self.dispatcher.dispatch(input, &self.map, &Active::NONE).into_iter().map(|hit| hit.command.id).collect()
    }

    /// The id of the command a key press outside the main window runs. The press is matched as
    /// the main window's presses are.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn command_of(&self, press: &Event) -> Option<String> {
        let input = KeyInput::from_egui(press, self.map.platform())?;
        self.dispatcher.layout().candidates(&input).into_iter().find_map(|chord| match self.map.lookup(&chord, false, &Active::NONE) {
            Lookup::Run(command) => Some(command.id),
            Lookup::Cycle(commands) => commands.into_iter().next().map(|command| command.id),
            _ => None,
        })
    }

    /// The shortcut a press records: the chord it runs, keeping the held Ctrl and Cmd as they are.
    fn recorded(&self, press: &Event, key: Key, modifiers: Modifiers, platform: Platform) -> KeyboardShortcut {
        let chord = KeyInput::from_egui(press, platform).and_then(|input| self.dispatcher.layout().resolve(&input));
        match chord {
            Some(chord) => KeyboardShortcut::new(Modifiers { shift: chord.mods.shift, ..modifiers }, chord.to_egui_shortcut().logical_key),
            None => KeyboardShortcut::new(modifiers, key),
        }
    }

    /// Whether a Shift-only Cut or Paste, read as Shift+Delete or Shift+Insert, runs nothing.
    pub(crate) fn shift_clipboard_is_free(&self, event: &Event) -> bool {
        let key = match event {
            Event::Cut => craft_keymap::Key::Delete,
            Event::Paste(_) => craft_keymap::Key::Insert,
            _ => return false,
        };
        matches!(self.map.lookup(&Chord::new(Mods::SHIFT, key), false, &Active::NONE), Lookup::None)
    }

    /// Another binding on the same shortcut as `b`.
    fn conflict(&self, b: &Binding) -> Option<&'static Binding> {
        let clash = self.map.conflicts_with(&CommandRef::new(b.id)).into_iter().find(|c| c.kind == ConflictKind::Clash)?;
        bindings().iter().find(|x| x.id == clash.b.0.id)
    }
}

/// Bind `b` to `value` ("" = no shortcut); the default is stored as no override.
pub fn rebind(keymap: &mut Keymap, b: &Binding, value: &str) {
    if b.default.unwrap_or("") == value {
        keymap.remove(b.id);
    } else {
        keymap.insert(b.id.to_owned(), value.to_owned());
    }
}

/// The bound shortcuts the browser must not act on, for the web build's `keydown` listener:
/// the keymap they were taken from and the shortcuts.
static CAPTURED: std::sync::Mutex<Option<(Keymap, Vec<KeyboardShortcut>)>> = std::sync::Mutex::new(None);

/// Refresh [`captured`] when the keymap changed (once per frame; cheap when it didn't).
pub fn sync_captured(keymap: &Keymap) {
    let mut c = CAPTURED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.as_ref().is_some_and(|(k, _)| k == keymap) {
        return;
    }
    // Copy, cut and paste stay with the browser: egui reads them as clipboard events.
    let clipboard = [Key::C, Key::V, Key::X];
    let keys: Vec<KeyboardShortcut> = active(keymap)
        .into_iter()
        .filter(|sc| {
            let fkey = sc.logical_key.name().strip_prefix('F').is_some_and(|n| n.parse::<u8>().is_ok());
            (sc.modifiers.command || sc.modifiers.alt || fkey) && !(sc.modifiers.command && !sc.modifiers.shift && clipboard.contains(&sc.logical_key))
        })
        .collect();
    log::debug!("browser shortcuts taken over: {}", keys.len());
    *c = Some((keymap.clone(), keys));
}

/// Whether a key press is one of the app's shortcuts the browser must leave alone.
pub fn captured(mods: Modifiers, key: Key) -> bool {
    let c = CAPTURED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    c.as_ref().is_some_and(|(_, keys)| keys.iter().any(|sc| sc.logical_key == key && mods.matches_logically(sc.modifiers)))
}

const CAPTURE: &str = "keymap_capture";

/// Whether the editor is waiting for a key combination (global shortcuts pause meanwhile).
pub fn capturing(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<usize>(Id::unique(CAPTURE))).is_some()
}

/// The first key press this frame as a shortcut.
fn pressed(ctx: &egui::Context, bindings: &Bindings) -> Option<KeyboardShortcut> {
    let platform = craft_keymap::egui::platform(ctx);
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } => Some(bindings.recorded(e, *key, *modifiers, platform)),
            _ => None,
        })
    })
}

/// The shortcut editor window.
pub fn window(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let tk = Tokens::get(ctx);
    if !app.ui.windows.shortcuts {
        return;
    }
    let capture_id = Id::unique(CAPTURE);
    let filter_id = Id::unique("keymap_filter");
    let mut capture: Option<usize> = ctx.data(|d| d.get_temp(capture_id));
    if let (Some(i), Some(sc)) = (capture, pressed(ctx, &app.bindings)) {
        if (sc.logical_key != Key::Escape || !sc.modifiers.is_none())
            && let Some(b) = bindings().get(i)
        {
            rebind(&mut app.ui.keymap, b, &format_shortcut(&sc));
        }
        capture = None;
    }
    app.bindings.sync(&app.ui.keymap, craft_keymap::egui::platform(ctx));
    let mut filter: String = ctx.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    let mut open = true;
    crate::windows::window("keyboard_shortcuts", tr("Keyboard shortcuts"), &mut open).default_width(480.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(ic::MAGNIFYING_GLASS).color(tk.text_dim));
            ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tr("Search commands or keys")).desired_width(300.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!app.ui.keymap.is_empty(), egui::Button::new(tr("Reset all"))).on_hover_text(tr("Restore every default shortcut")).clicked() {
                    app.ui.keymap.clear();
                }
            });
        });
        let clashes = bindings().iter().filter(|b| app.bindings.conflict(b).is_some()).count();
        if clashes > 0 {
            ui.label(
                RichText::new(format!("{} {}", ic::WARNING, trf("{n} items share a shortcut; the first one listed wins.", &[("n", &clashes)])))
                    .color(tk.danger),
            );
        }
        ui.add_space(4.0);
        let needle = filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("keymap_grid").num_columns(3).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                for (i, b) in bindings().iter().enumerate() {
                    let current = shortcut(&app.ui.keymap, b);
                    if !needle.is_empty()
                        && ![b.label, b.group, tr(b.label), tr(b.group), b.id, current.unwrap_or("")].iter().any(|t| t.to_lowercase().contains(&needle))
                    {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr(b.label));
                        ui.label(RichText::new(tr(b.group)).size(10.5).color(tk.text_dim));
                    });
                    let waiting = capture == Some(i);
                    let text = if waiting { tr("Press a key...") } else { current.unwrap_or("-") };
                    let key = egui::Button::new(RichText::new(text).monospace()).selected(waiting).min_size(vec2(130.0, 0.0));
                    if ui.add(key).on_hover_text(tr("Click, then press the new shortcut (Esc cancels)")).clicked() {
                        capture = if waiting { None } else { Some(i) };
                    }
                    let bound = current.is_some();
                    ui.horizontal(|ui| {
                        if let Some(other) = app.bindings.conflict(b) {
                            ui.label(RichText::new(ic::WARNING).color(tk.danger))
                                .on_hover_text(trf("Also bound to {item} ({menu})", &[("item", &tr(other.label)), ("menu", &tr(other.group))]));
                        }
                        if bound && icon_button(ui, ic::X, tr("Remove the shortcut"), false, 18.0).clicked() {
                            rebind(&mut app.ui.keymap, b, "");
                        }
                        if app.ui.keymap.contains_key(b.id)
                            && icon_button(
                                ui,
                                ic::ARROW_COUNTER_CLOCKWISE,
                                &trf("Reset to {shortcut}", &[("shortcut", &b.default.unwrap_or(tr("none")))]),
                                false,
                                18.0,
                            )
                            .clicked()
                        {
                            app.ui.keymap.remove(b.id);
                        }
                    });
                    ui.end_row();
                }
            });
        });
        ui.add_space(4.0);
        section(ui, "keymap_nav", tr("Viewport and brush (fixed)"), false, |ui| {
            egui::Grid::new("keymap_fixed").striped(true).show(ui, |ui| {
                for (k, v) in [
                    (tr("Alt + LMB drag"), tr("Orbit")),
                    (tr("MMB drag / Alt + MMB"), tr("Pan")),
                    (tr("Wheel / Alt + RMB drag"), tr("Zoom")),
                    (tr("LMB"), tr("Paint with the current tool")),
                    ("[ / ]", tr("Brush size down / up")),
                    (tr("Shift + wheel"), tr("Brush size")),
                    ("Esc", tr("Cancel the stroke")),
                ] {
                    ui.label(RichText::new(k).strong());
                    ui.label(v);
                    ui.end_row();
                }
            });
        });
    });
    app.ui.windows.shortcuts = open;
    if !open {
        capture = None;
    }
    ctx.data_mut(|d| {
        d.insert_temp(filter_id, filter);
        match capture {
            Some(i) => {
                d.insert_temp(capture_id, i);
            }
            None => {
                d.remove::<usize>(capture_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_leaves_app_shortcuts_alone_but_keeps_the_clipboard() {
        let mut keymap = Keymap::new();
        keymap.insert("file.close".into(), "Alt+W".into());
        sync_captured(&keymap);
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        assert!(captured(ctrl, Key::S) && captured(ctrl, Key::O) && captured(ctrl, Key::D));
        assert!(captured(Modifiers::NONE, Key::F8) && captured(Modifiers::ALT, Key::W));
        assert!(!captured(ctrl, Key::C) && !captured(ctrl, Key::V) && !captured(ctrl, Key::X), "clipboard keys reach the browser");
        assert!(!captured(Modifiers::NONE, Key::M) && !captured(Modifiers::NONE, Key::F), "plain keys need no capture");
    }

    #[test]
    fn shortcuts_round_trip_through_parse_and_format() {
        for b in bindings() {
            if let Some(s) = b.default {
                let chord = parse(s).unwrap_or_else(|| panic!("{} has an unparsable shortcut {s}", b.id));
                assert_eq!(format_shortcut(&chord.to_egui_shortcut()), s);
            }
        }
        for s in ["Ctrl+Shift+Alt+F12", "Alt+OpenBracket", "Shift+Delete", "Ctrl+9"] {
            assert_eq!(parse(s).map(|chord| format_shortcut(&chord.to_egui_shortcut())).as_deref(), Some(s));
        }
        assert!(parse("Ctrl+").is_none() && parse("Ctrl+NoSuchKey").is_none());
    }

    fn bindings_for(keymap: &Keymap) -> Bindings {
        let mut out = Bindings::default();
        out.sync(keymap, Platform::Windows);
        out
    }

    fn binding(id: &str) -> &'static Binding {
        bindings().iter().find(|b| b.id == id).unwrap()
    }

    #[test]
    fn default_shortcuts_do_not_conflict() {
        let resolved = bindings_for(&Keymap::new());
        for b in bindings() {
            assert!(resolved.conflict(b).is_none(), "{} shares its default shortcut", b.id);
        }
    }

    #[test]
    fn rebinding_detects_conflicts_and_resets() {
        let (save, undo) = (binding("file.save"), binding("edit.undo"));
        let mut keymap = Keymap::new();
        rebind(&mut keymap, save, "Ctrl+Z");
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.conflict(save).map(|b| b.id), Some("edit.undo"));
        assert_eq!(resolved.conflict(undo).map(|b| b.id), Some("file.save"));
        assert_eq!(shortcut_of(&keymap, "file.save"), Some("Ctrl+Z"));
        // Unbinding clears the conflict; binding back to the default drops the override.
        rebind(&mut keymap, save, "");
        assert_eq!(shortcut_of(&keymap, "file.save"), None);
        assert!(bindings_for(&keymap).conflict(undo).is_none());
        rebind(&mut keymap, save, "Ctrl+S");
        assert!(keymap.is_empty());
    }

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn press_at(key: Key, physical: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers }
    }

    fn dispatched(bindings: &mut Bindings, events: Vec<Event>, modifiers: Modifiers) -> Vec<String> {
        let mut input = egui::InputState::default();
        input.modifiers = modifiers;
        input.events = events;
        bindings.dispatch(&mut input)
    }

    #[test]
    fn a_stored_shortcut_on_a_shifted_symbol_runs_on_that_press() {
        for (stored, key, physical, modifiers) in [
            ("Shift+Colon", Key::Colon, Key::Semicolon, Modifiers::SHIFT),
            ("Shift+Exclamationmark", Key::Exclamationmark, Key::Num1, Modifiers::SHIFT),
            ("Shift+Questionmark", Key::Questionmark, Key::Slash, Modifiers::SHIFT),
            ("Ctrl+Shift+Plus", Key::Plus, Key::Equals, Modifiers::COMMAND | Modifiers::SHIFT),
        ] {
            let mut keymap = Keymap::new();
            keymap.insert("file.save".into(), stored.into());
            let mut resolved = bindings_for(&keymap);
            assert_eq!(dispatched(&mut resolved, vec![press_at(key, physical, modifiers)], modifiers), ["file.save"], "{stored}");
            assert_eq!(resolved.command_of(&press_at(key, physical, modifiers)).as_deref(), Some("file.save"), "{stored} in a floating panel");
        }
    }

    #[test]
    fn a_recorded_shifted_symbol_is_the_chord_it_runs() {
        let resolved = bindings_for(&Keymap::new());
        let recorded = |key, physical, modifiers| format_shortcut(&resolved.recorded(&press_at(key, physical, modifiers), key, modifiers, Platform::Windows));
        assert_eq!(recorded(Key::Colon, Key::Semicolon, Modifiers::SHIFT), "Colon");
        // The layout needs Shift for this slash; the key at that position is not a symbol.
        assert_eq!(recorded(Key::Slash, Key::Num7, Modifiers::SHIFT), "Slash");
        assert_eq!(recorded(Key::E, Key::E, Modifiers::COMMAND | Modifiers::SHIFT), "Ctrl+Shift+E");
    }

    #[test]
    fn a_floating_panel_matches_presses_as_the_main_window_does() {
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "Equals".into());
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.command_of(&press(Key::Plus, Modifiers::NONE)).as_deref(), Some("file.save"), "+ stands for =");
    }

    #[test]
    fn a_floating_panel_leaves_a_bound_shift_insert_to_its_binding() {
        let paste = || Event::Paste("text".into());
        let free = bindings_for(&Keymap::new());
        let event = crate::menus::forwarded_press(&free, paste(), Modifiers::SHIFT);
        assert_eq!(free.command_of(&event).as_deref(), Some("layer.paste"));
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "Shift+Insert".into());
        keymap.insert("file.saveAs".into(), "Shift+Delete".into());
        let bound = bindings_for(&keymap);
        let event = crate::menus::forwarded_press(&bound, paste(), Modifiers::SHIFT);
        assert_eq!(bound.command_of(&event).as_deref(), Some("file.save"));
        let event = crate::menus::forwarded_press(&bound, Event::Cut, Modifiers::SHIFT);
        assert_eq!(bound.command_of(&event).as_deref(), Some("file.saveAs"));
    }

    #[test]
    fn a_press_runs_the_command_bound_to_exactly_that_chord() {
        let resolved = bindings_for(&Keymap::new());
        let run = |mods: Modifiers, key: Key| resolved.command_of(&press(key, mods));
        assert_eq!(run(Modifiers::COMMAND, Key::S).as_deref(), Some("file.save"));
        assert_eq!(run(Modifiers::COMMAND | Modifiers::SHIFT, Key::S).as_deref(), Some("file.saveAs"));
        assert_eq!(run(Modifiers::SHIFT, Key::C).as_deref(), Some("viewport.previousChannel"));
        assert_eq!(run(Modifiers::NONE, Key::C).as_deref(), Some("viewport.nextChannel"));
        assert_eq!(run(Modifiers::ALT, Key::C), None, "Alt+C is not bound");
        assert_eq!(run(Modifiers::SHIFT, Key::M), None, "Shift+M is not bound");
    }

    #[test]
    fn a_stored_shortcut_replaces_the_default_and_an_empty_one_unbinds() {
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "F5".into());
        keymap.insert("edit.undo".into(), String::new());
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.command_of(&press(Key::F5, Modifiers::NONE)).as_deref(), Some("file.save"));
        assert_eq!(resolved.command_of(&press(Key::S, Modifiers::COMMAND)), None);
        assert_eq!(resolved.command_of(&press(Key::Z, Modifiers::COMMAND)), None);
    }

    #[test]
    fn ctrl_in_the_notation_is_the_command_key_on_macos() {
        let mut resolved = Bindings::default();
        resolved.sync(&Keymap::new(), Platform::Mac);
        let cmd = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };
        assert_eq!(resolved.command_of(&press(Key::S, cmd)).as_deref(), Some("file.save"));
        assert_eq!(resolved.command_of(&press(Key::S, Modifiers::CTRL)), None, "the Control key is another key");
    }

    #[test]
    fn the_first_item_listed_wins_a_shared_shortcut() {
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "Ctrl+Z".into());
        let resolved = bindings_for(&keymap);
        let first = bindings().iter().find(|b| matches!(b.id, "file.save" | "edit.undo")).unwrap().id;
        assert_eq!(resolved.command_of(&press(Key::Z, Modifiers::COMMAND)).as_deref(), Some(first));
    }
}
