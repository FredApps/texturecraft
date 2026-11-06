//! The menu bar and global shortcuts. The menu catalog follows the menu layout of a standard
//! texture-painting workflow, with stable ids; an item is live when the engine (or the shell, for `ui.*` ids) implements
//! its id. The same catalog drives `docs/parity.md`.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::state::{Mode, ViewLayout};
use crate::theme::Tokens;

/// One menu entry. `label == "-"` is a separator; `sub` names a submenu group.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub menu: &'static str,
    pub sub: Option<&'static str>,
    pub label: &'static str,
    pub id: &'static str,
    pub shortcut: Option<&'static str>,
}

const fn it(menu: &'static str, label: &'static str, id: &'static str, shortcut: Option<&'static str>) -> Item {
    Item { menu, sub: None, label, id, shortcut }
}
const fn sub(menu: &'static str, sub: &'static str, label: &'static str, id: &'static str, shortcut: Option<&'static str>) -> Item {
    Item { menu, sub: Some(sub), label, id, shortcut }
}
const fn sep(menu: &'static str) -> Item {
    Item { menu, sub: None, label: "-", id: "", shortcut: None }
}

pub const TOP: [&str; 6] = ["File", "Edit", "Mode", "Window", "Viewport", "Help"];

/// The menu catalog (the target menu tree with our ids).
pub const CATALOG: &[Item] = &[
    it("File", "New project...", "ui.newProject", Some("Ctrl+N")),
    it("File", "Open...", "file.open", Some("Ctrl+O")),
    it("File", "Open sample project...", "project.sample", None),
    it("File", "Close", "file.close", Some("Ctrl+F4")),
    sep("File"),
    it("File", "Save", "file.save", Some("Ctrl+S")),
    it("File", "Save as...", "file.saveAs", Some("Ctrl+Shift+S")),
    it("File", "Save a copy...", "file.saveCopy", None),
    it("File", "Remove unused resources...", "file.removeUnused", None),
    sep("File"),
    it("File", "Import resources...", "resource.import", None),
    it("File", "Import MaterialX material...", "file.importMaterialX", None),
    it("File", "Import material...", "material.load", None),
    it("File", "Export Textures...", "ui.export", Some("Ctrl+Shift+E")),
    it("File", "Export Mesh...", "file.exportMesh", None),
    sep("File"),
    it("File", "Quit", "ui.quit", Some("Ctrl+Q")),
    it("Edit", "Undo", "edit.undo", Some("Ctrl+Z")),
    it("Edit", "Redo", "edit.redo", Some("Ctrl+Y")),
    sep("Edit"),
    it("Edit", "Project Configuration...", "ui.projectConfig", None),
    it("Edit", "Reimport mesh", "file.reimportMesh", Some("Ctrl+Shift+R")),
    it("Edit", "Settings...", "ui.settings", None),
    it("Mode", "Bake Mesh Maps", "ui.mode.bake", Some("F8")),
    it("Mode", "Painting", "ui.mode.paint", Some("F9")),
    it("Mode", "Rendering (path traced)", "ui.mode.render", Some("F10")),
    sub("Window", "Views", "Assets", "ui.view.assets", None),
    sub("Window", "Views", "Common Settings", "ui.view.bakeSettings", None),
    sub("Window", "Views", "Display Settings", "ui.view.display", None),
    sub("Window", "Views", "History", "ui.view.history", None),
    sub("Window", "Views", "Layers", "ui.view.layers", None),
    sub("Window", "Views", "Log", "ui.view.log", None),
    sub("Window", "Views", "Mesh Map Bakers", "ui.view.meshMapBakers", None),
    sub("Window", "Views", "Mesh Map Settings", "ui.view.bakeMaps", None),
    sub("Window", "Views", "Properties", "ui.view.properties", None),
    sub("Window", "Views", "Shader Settings", "ui.view.shader", None),
    sub("Window", "Views", "Texture Sets", "ui.view.textureSetList", None),
    sub("Window", "Views", "Texture Set Settings", "ui.view.textureSetSettings", None),
    sub("Window", "Save UI Layout", "Save current layout...", "ui.layout.save", None),
    it("Window", "Load UI Layout", "ui.layout.load", None),
    it("Window", "Hide UI", "ui.hideUi", Some("Tab")),
    it("Window", "Reset UI", "ui.resetUi", None),
    it("Viewport", "Display material", "viewport.displayMaterial", Some("M")),
    it("Viewport", "Display next channel", "viewport.nextChannel", Some("C")),
    it("Viewport", "Display previous channel", "viewport.previousChannel", Some("Shift+C")),
    it("Viewport", "Display next mesh map", "viewport.nextMeshMap", Some("B")),
    it("Viewport", "Display previous mesh map", "viewport.previousMeshMap", Some("Shift+B")),
    it("Viewport", "UV checker", "ui.uvChecker", None),
    sep("Viewport"),
    it("Viewport", "Show entire mesh", "viewport.frame", Some("F")),
    sep("Viewport"),
    it("Viewport", "3D/2D views", "ui.layout.split", Some("F1")),
    it("Viewport", "3D view only", "ui.layout.3d", Some("F2")),
    it("Viewport", "2D view only", "ui.layout.2d", Some("F3")),
    sep("Viewport"),
    it("Viewport", "Enable quick mask", "viewport.quickMask", Some("Y")),
    it("Viewport", "Edit quick mask", "viewport.quickMask.edit", Some("U")),
    it("Viewport", "Invert quick mask", "viewport.quickMask.invert", Some("Ctrl+Shift+I")),
    it("Help", "Documentation...", "ui.help.docs", None),
    it("Help", "Shortcuts list...", "ui.help.shortcuts", None),
    it("Help", "Join the community (Discord)...", "ui.help.discord", None),
    it("Help", "ArtCraft website...", "ui.help.website", None),
    it("Help", "Report a bug...", "ui.help.bug", None),
    sep("Help"),
    it("Help", "Show log...", "ui.view.log", None),
    it("Help", "About TextureCraft...", "ui.about", None),
];

/// Ids the shell handles itself.
fn is_ui(id: &str) -> bool {
    id.starts_with("ui.")
}

/// Whether an item can run now.
pub fn enabled(app: &TextureCraftApp, id: &str) -> bool {
    if let Some(panel) = crate::docking::view_panel(id) {
        return panel.available_in(app.ui.mode);
    }

    if app.ui.mode != Mode::Painting && (id.starts_with("tool.") || id.starts_with("viewport.quickMask")) {
        return false;
    }

    if is_ui(id) {
        return true;
    }
    texturecraft_engine::find(id).is_some_and(|c| (c.enabled)(&app.session).is_ok())
}

/// Whether an id is implemented at all (parity report).
pub fn implemented(id: &str) -> bool {
    is_ui(id) || texturecraft_engine::find(id).is_some()
}

/// Invoke a menu item (menus, shortcuts, palette, control channel `ui.menu.invoke`).
pub fn invoke(app: &mut TextureCraftApp, ctx: &egui::Context, id: &str) {
    if !enabled(app, id) {
        return;
    }

    if matches!(id, "ui.newProject" | "file.open" | "file.close" | "project.sample")
        && crate::windows::ask_to_save(app, crate::state::Unsaved::Command(id.into()))
    {
        return;
    }
    if let Some(panel) = crate::docking::view_panel(id) {
        app.ui.dock.toggle(panel);
        return;
    }
    match id {
        "ui.newProject" => app.ui.windows.new_project = true,
        "ui.export" => app.ui.windows.export = true,
        "ui.quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        "ui.projectConfig" => app.ui.dock.focus(crate::docking::Panel::Display),
        "ui.settings" => app.ui.windows.settings = true,
        "ui.mode.bake" => app.ui.mode = Mode::Baking,
        "ui.mode.paint" => app.ui.mode = Mode::Painting,
        "ui.mode.render" => app.ui.mode = Mode::Rendering,
        "ui.hideUi" => app.ui.hide_ui = !app.ui.hide_ui,
        "ui.resetUi" => app.ui.adopt_layout(crate::UiState::default()),
        "ui.layout.save" => crate::windows::layout_file(app, true),
        "ui.layout.load" => crate::windows::layout_file(app, false),
        "ui.uvChecker" => {
            app.run("viewport.view", json!({"mode": "uvChecker"}));
        }
        "ui.layout.split" => app.ui.layout = ViewLayout::Split,
        "ui.layout.3d" => app.ui.layout = ViewLayout::Only3D,
        "ui.layout.2d" => app.ui.layout = ViewLayout::Only2D,
        "ui.help.shortcuts" => app.ui.windows.shortcuts = true,
        "ui.about" => app.ui.windows.about = true,
        "ui.help.discord" => ctx.open_url(egui::OpenUrl::new_tab("https://discord.gg/artcraft")),
        "ui.help.website" => ctx.open_url(egui::OpenUrl::new_tab("https://getartcraft.com/apps/texturecraft")),
        "ui.help.docs" => ctx.open_url(egui::OpenUrl::new_tab("https://github.com/storytold/texturecraft#readme")),
        "ui.help.bug" => ctx.open_url(egui::OpenUrl::new_tab("https://github.com/storytold/texturecraft/issues")),
        _ => {
            if crate::windows::file_action(app, ctx, id) {
                return;
            }
            app.run(id, Value::Null);
        }
    }
}

/// Parse "Ctrl+Shift+E" style shortcuts.
pub fn parse_shortcut(s: &str) -> Option<KeyboardShortcut> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part {
            "Ctrl" | "Cmd" => mods |= Modifiers::COMMAND,
            "Shift" => mods |= Modifiers::SHIFT,
            "Alt" => mods |= Modifiers::ALT,
            k => key = Key::from_name(k),
        }
    }
    key.map(|k| KeyboardShortcut::new(mods, k))
}

/// The menus, then the mode label at the right of `ui` (the strip `titlebar::show` lays out).
pub fn menu_bar(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let tk = Tokens::get(ui.ctx());
    egui::MenuBar::new().ui(ui, |ui| {
        let ctx = ui.ctx().clone();
        for top in TOP {
            ui.menu_button(tr(top), |ui| {
                ui.set_min_width(220.0);
                let mut current_sub: Option<&str> = None;
                let items: Vec<Item> = CATALOG.iter().filter(|i| i.menu == top).copied().collect();
                for item in &items {
                    if let Some(s) = item.sub {
                        if current_sub == Some(s) {
                            continue;
                        }
                        current_sub = Some(s);
                        ui.menu_button(tr(s), |ui| {
                            for x in items.iter().filter(|x| x.sub == Some(s)) {
                                menu_item(app, &ctx, ui, x);
                            }
                        });
                        continue;
                    }
                    if item.label == "-" {
                        ui.separator();
                        continue;
                    }
                    menu_item(app, &ctx, ui, item);
                }
            });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mode = match app.ui.mode {
                Mode::Painting => tr("Painting"),
                Mode::Baking => tr("Baking"),
                Mode::Rendering => tr("Rendering"),
            };
            ui.label(egui::RichText::new(mode).size(11.0).color(tk.text_dim));
        });
    });
}

fn menu_item(app: &mut TextureCraftApp, ctx: &egui::Context, ui: &mut egui::Ui, item: &Item) {
    let on = enabled(app, item.id);
    let mut b = egui::Button::new(tr(item.label));
    if let Some(sc) = crate::keymap::shortcut_of(&app.ui.keymap, item.id) {
        b = b.shortcut_text(sc);
    }
    let checked = match item.id {
        "ui.mode.paint" => Some(app.ui.mode == Mode::Painting),
        "ui.mode.bake" => Some(app.ui.mode == Mode::Baking),
        "ui.mode.render" => Some(app.ui.mode == Mode::Rendering),
        id => crate::docking::view_panel(id).map(|p| app.ui.dock.is_open(p)),
    };
    if let Some(c) = checked {
        b = b.selected(c);
    }
    if ui.add_enabled(on, b).clicked() {
        invoke(app, ctx, item.id);
        ui.close();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if item.id == "file.open" {
        ui.add_enabled_ui(!app.ui.recent.is_empty(), |ui| {
            ui.menu_button(tr("Recent projects"), |ui| {
                for path in app.ui.recent.clone() {
                    if ui.button(&path).clicked() {
                        crate::windows::open_path(app, &path);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(tr("Clear recent projects")).clicked() {
                    app.ui.recent.clear();
                    ui.close();
                }
            });
        });
    }
}

/// Global shortcuts: the catalog's and every engine command's (defaults or the user's keymap),
/// plus the fixed brush keys.
/// Keys pressed in a floating panel's own OS window run the shortcuts the main window's keys do.
#[cfg(not(target_arch = "wasm32"))]
pub fn forwarded_shortcuts(app: &mut TextureCraftApp, ctx: &egui::Context, events: Vec<egui::Event>, held: Modifiers) {
    if crate::keymap::capturing(ctx) {
        return;
    }
    let active = crate::keymap::active(&app.ui.keymap);
    for event in events {
        let event = clipboard_key(&event, held).unwrap_or(event);
        let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event else { continue };
        let Some((id, _)) = active.iter().find(|(_, sc)| sc.logical_key == key && modifiers.matches_logically(sc.modifiers)) else { continue };
        invoke(app, ctx, effect_command(app, id));
    }
}

/// Layer shortcuts act on the selected effect instead of its layer (copy, cut, duplicate,
/// remove), and paste a copied effect onto the selection.
fn effect_command<'a>(app: &TextureCraftApp, id: &'a str) -> &'a str {
    let effect = matches!(app.session.target, texturecraft_engine::StackTarget::Effect(_));
    match id {
        "layer.copy" if effect => "effect.copy",
        "layer.cut" if effect => "effect.cut",
        "layer.duplicate" if effect => "effect.duplicate",
        "layer.remove" if effect => "effect.remove",
        "layer.paste" if enabled(app, "effect.paste") => "effect.paste",
        _ => id,
    }
}

pub fn shortcuts(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if crate::keymap::capturing(ctx) || (ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input()) {
        return;
    }

    ctx.input_mut(|input| {
        let keys: Vec<_> = input.events.iter().filter_map(|event| clipboard_key(event, input.modifiers)).collect();
        input.events.extend(keys);
    });

    if !ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))) {
        return;
    }

    let active_shortcuts = crate::keymap::active(&app.ui.keymap);
    let mut commands_to_invoke: Vec<&str> = Vec::new();
    ctx.input_mut(|input| {
        for (id, shortcut) in &active_shortcuts {
            if input.consume_shortcut(shortcut) {
                commands_to_invoke.push(*id);
            }
        }

        let size_down = input.consume_key(Modifiers::NONE, Key::OpenBracket);
        let size_up = input.consume_key(Modifiers::NONE, Key::CloseBracket);
        if app.ui.mode == Mode::Painting && (size_down || size_up) {
            let scale = if size_up { 1.15 } else { 1.0 / 1.15 };
            app.session.tool.brush.size = (app.session.tool.brush.size * scale).clamp(0.0005, 0.25);
        }

        if app.ui.mode == Mode::Painting && input.consume_key(Modifiers::NONE, Key::Escape) {
            app.session.cancel_stroke();
            app.session.tool.path.points.clear();
            app.session.tool.path.editing = None;
        }
    });

    for id in commands_to_invoke {
        if matches!(id, "layer.copy" | "layer.cut")
            && let Some(layer) = app.session.selected_layer().and_then(|layer| app.session.doc.project.layers.get(&layer))
        {
            // Ctrl+V reaches the app only while the system clipboard holds text or an image
            ctx.copy_text(layer.name.clone());
        }
        invoke(app, ctx, effect_command(app, id));
    }
}

/// The desktop and web shells turn Ctrl+C, Ctrl+X and Ctrl+V (and Ctrl+Insert, Shift+Delete,
/// Shift+Insert) into clipboard events instead of key presses. Outside a text field they are the
/// shortcuts bound to Ctrl+C, Ctrl+X and Ctrl+V (copy, cut and paste a layer by default).
fn clipboard_key(event: &egui::Event, held: Modifiers) -> Option<egui::Event> {
    let key = match event {
        egui::Event::Copy => Key::C,
        egui::Event::Cut => Key::X,
        egui::Event::Paste(_) | egui::Event::PasteImage(_) => Key::V,
        _ => return None,
    };
    Some(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: held | Modifiers::COMMAND })
}
