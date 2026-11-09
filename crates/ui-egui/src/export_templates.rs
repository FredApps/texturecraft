//! Export dialog › Output templates: create and edit export presets. Built-in presets are
//! read-only; the selected user preset is edited as a draft that is saved through
//! `export.preset.save` whenever it validates, so a half-typed file name never reaches disk.

use std::sync::LazyLock;

use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::formats::export::{FILE_TOKENS, MAX_PRESET_NAME, sanitize_file_stem};
use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef, VirtualMap};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::theme::{Tokens, caption};
use crate::widgets::{chip, icon_button};

const ICON: f32 = 20.0;
const LIST_WIDTH: f32 = 190.0;
const INPUTS_WIDTH: f32 = 170.0;
const RENAME_ID: &str = "exp_rename";
const RGB: [&str; 3] = ["R", "G", "B"];

/// An output file's channel layout.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    Rgb,
    Rgba,
    Gray,
    GrayAlpha,
}

impl Layout {
    const ALL: [Layout; 4] = [Layout::Rgb, Layout::Rgba, Layout::Gray, Layout::GrayAlpha];

    fn label(self) -> &'static str {
        match self {
            Layout::Rgb => "RGB",
            Layout::Rgba => "RGBA",
            Layout::Gray => n_("Gray"),
            Layout::GrayAlpha => n_("Gray + A"),
        }
    }

    fn dests(self) -> &'static [Dest] {
        match self {
            Layout::Rgb => &[Dest::R, Dest::G, Dest::B],
            Layout::Rgba => &[Dest::R, Dest::G, Dest::B, Dest::A],
            Layout::Gray => &[Dest::L],
            Layout::GrayAlpha => &[Dest::L, Dest::A],
        }
    }

    fn of(map: &MapSpec) -> Layout {
        let has = |d: Dest| map.channels.iter().any(|(x, _)| *x == d);
        match (has(Dest::L), has(Dest::A)) {
            (true, true) => Layout::GrayAlpha,
            (true, false) => Layout::Gray,
            (false, true) => Layout::Rgba,
            (false, false) => Layout::Rgb,
        }
    }
}

/// The input maps palette: (group, [(label, source)]).
type Palette = [(&'static str, Vec<(&'static str, SourceRef)>); 4];

static PALETTE: LazyLock<Palette> = LazyLock::new(|| {
    [
        (n_("Texture set channels"), Channel::ALL.iter().map(|&channel| (channel.label(), SourceRef::Channel { channel, component: 0 })).collect()),
        (n_("Converted maps"), VirtualMap::ALL.iter().map(|&map| (map.label(), SourceRef::Virtual { map, component: 0 })).collect()),
        (n_("Mesh maps"), BakedMap::ALL.iter().map(|&map| (crate::i18n::baked_map_label(map), SourceRef::Baked { map, component: 0 })).collect()),
        (n_("Constants"), vec![(n_("Black"), SourceRef::Constant(0.0)), (n_("Gray 50%"), SourceRef::Constant(0.5)), (n_("White"), SourceRef::Constant(1.0))]),
    ]
});

fn source_label(s: &SourceRef) -> &'static str {
    match s {
        SourceRef::Channel { channel, .. } => channel.label(),
        SourceRef::Virtual { map, .. } => map.label(),
        SourceRef::Baked { map, .. } => crate::i18n::baked_map_label(*map),
        SourceRef::Constant(_) => n_("Constant"),
    }
}

/// `s` reading component `k` (only 3-component sources have more than one).
fn with_component(s: &SourceRef, k: u8) -> SourceRef {
    let k = if s.components() == 3 { k.min(2) } else { 0 };
    let mut s = s.clone();
    if let SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } = &mut s {
        *component = k;
    }
    s
}

/// Same input map (any component), or the same constant.
fn same_source(a: &SourceRef, b: &SourceRef) -> bool {
    match (a, b) {
        (SourceRef::Constant(x), SourceRef::Constant(y)) => (x - y).abs() < 1e-4,
        _ => with_component(a, 0) == with_component(b, 0),
    }
}

/// The component a channel reads by default: R, G, B their own, L and A the first.
fn slot_component(d: Dest) -> u8 {
    match d {
        Dest::G => 1,
        Dest::B => 2,
        _ => 0,
    }
}

fn is_rgb(d: Dest) -> bool {
    matches!(d, Dest::R | Dest::G | Dest::B)
}

/// Switch `map` to `layout`, keeping the sources of channels both layouts have; the color
/// source moves between R, G, B and L.
fn set_layout(map: &mut MapSpec, layout: Layout) {
    let get = |d: Dest| map.channels.iter().find(|(x, _)| *x == d).map(|(_, s)| s.clone());
    let color = get(Dest::R).or_else(|| get(Dest::L)).unwrap_or(SourceRef::Constant(0.0));
    let channels = layout
        .dests()
        .iter()
        .map(|&d| {
            let src = match d {
                Dest::A => get(d).unwrap_or(SourceRef::Constant(1.0)),
                _ => get(d).unwrap_or_else(|| with_component(&color, slot_component(d))),
            };
            (d, src)
        })
        .collect();
    map.channels = channels;
}

fn new_map(layout: Layout) -> MapSpec {
    let mut map = MapSpec { file: "$mesh_$textureSet_Output".into(), format: FileFormat::Png, bit_depth: 8, channels: Vec::new() };
    set_layout(&mut map, layout);
    map
}

/// An input map dropped on channel `dest`: a color or normal input dropped on R, G or B fills
/// all three.
fn drop_input(map: &mut MapSpec, dest: Dest, input: &SourceRef) {
    let fill_rgb = input.components() == 3 && is_rgb(dest);
    for (d, s) in &mut map.channels {
        if *d == dest || (fill_rgb && is_rgb(*d)) {
            *s = with_component(input, slot_component(*d));
        }
    }
}

/// The saved (built-in or user) preset selected in the dialog.
fn selected(app: &TextureCraftApp) -> Option<ExportPreset> {
    app.session.export_preset(&app.ui.export.preset)
}

/// Keep the draft on the selected user preset; a valid draft that differs from the saved preset
/// was changed elsewhere (an agent) and reloads, an invalid one keeps the user's edit.
fn sync_draft(app: &mut TextureCraftApp) {
    let e = &mut app.ui.export;
    match app.session.export_presets.list.iter().find(|p| p.name == e.preset) {
        None => e.draft = None,
        Some(saved) => {
            if e.draft.as_ref().is_none_or(|d| d.name != saved.name || (d != saved && d.validate().is_ok())) {
                e.draft = Some(saved.clone());
            }
        }
    }
}

enum ListAction {
    Select(String),
    New,
    Duplicate,
    StartRename(String),
    Rename(String),
    Remove,
    Import,
    Export,
}

enum MapAction {
    Add(Layout),
    Up(usize),
    Down(usize),
    Remove(usize),
}

/// The Output templates tab: template list, the selected template's outputs, input maps.
pub fn tab(app: &mut TextureCraftApp, ui: &mut Ui, builtins: &[ExportPreset]) {
    sync_draft(app);
    let height = ui.available_height();
    let mut list_action = None;
    ui.vertical(|ui| {
        ui.set_width(LIST_WIDTH);
        ui.set_height(height);
        list_action = preset_list(app, ui, builtins);
    });
    ui.separator();
    ui.vertical(|ui| {
        // Leave room for the input maps column and two separators.
        ui.set_width((ui.available_width() - INPUTS_WIDTH - 5.0 * ui.spacing().item_spacing.x - 4.0).max(200.0));
        ui.set_height(height);
        outputs(app, ui);
    });
    ui.separator();
    ui.vertical(|ui| {
        ui.set_width(INPUTS_WIDTH);
        ui.set_height(height);
        input_maps(ui);
    });
    if let Some(a) = list_action {
        apply(app, ui.ctx(), a);
    }
}

fn preset_list(app: &mut TextureCraftApp, ui: &mut Ui, builtins: &[ExportPreset]) -> Option<ListAction> {
    let mut act = None;
    let sel = app.ui.export.preset.clone();
    let user = app.ui.export.draft.is_some();
    ui.label(caption(tr("Output templates")));
    ui.horizontal(|ui| {
        let tool = |ui: &mut Ui, enabled: bool, icon: &str, tip: &str| ui.add_enabled_ui(enabled, |ui| icon_button(ui, icon, tip, false, ICON)).inner.clicked();
        if tool(ui, true, ic::PLUS, tr("New template")) {
            act = Some(ListAction::New);
        }
        if tool(ui, true, ic::COPY, tr("Duplicate the selected template")) {
            act = Some(ListAction::Duplicate);
        }
        if tool(ui, user, ic::PENCIL_SIMPLE, tr("Rename the selected template")) {
            act = Some(ListAction::StartRename(sel.clone()));
        }
        if tool(ui, user, ic::TRASH, tr("Delete the selected template")) {
            act = Some(ListAction::Remove);
        }
        ui.separator();
        if tool(ui, true, ic::DOWNLOAD_SIMPLE, tr("Import a template (.json)")) {
            act = Some(ListAction::Import);
        }
        if tool(ui, true, ic::UPLOAD_SIMPLE, tr("Export the selected template (.json)")) {
            act = Some(ListAction::Export);
        }
    });
    ui.add_space(2.0);
    let names: Vec<String> = app.session.export_presets.list.iter().map(|p| p.name.clone()).collect();
    egui::ScrollArea::vertical().id_salt("exp_presets").auto_shrink([false; 2]).show(ui, |ui| {
        for p in builtins {
            let r =
                ui.selectable_label(p.name == sel, format!("{}  {}", ic::LOCK_SIMPLE, p.name)).on_hover_text(tr("Built-in (read-only): duplicate it to edit"));
            if r.clicked() {
                act = Some(ListAction::Select(p.name.clone()));
            }
        }
        if !names.is_empty() {
            ui.separator();
        }
        for name in names {
            match app.ui.export.renaming.as_mut().filter(|_| name == sel) {
                Some(buf) => {
                    let r = ui.add(egui::TextEdit::singleline(buf).id(egui::Id::unique(RENAME_ID)).desired_width(f32::INFINITY));
                    if r.lost_focus() {
                        act = Some(if ui.input(|i| i.key_pressed(egui::Key::Escape)) { ListAction::Select(name) } else { ListAction::Rename(buf.clone()) });
                    }
                }
                None => {
                    let r = ui.selectable_label(name == sel, format!("{}  {name}", ic::USER)).on_hover_text(tr("Double-click to rename"));
                    if r.double_clicked() {
                        act = Some(ListAction::StartRename(name));
                    } else if r.clicked() {
                        act = Some(ListAction::Select(name));
                    }
                }
            }
        }
    });
    act
}

fn apply(app: &mut TextureCraftApp, ctx: &egui::Context, act: ListAction) {
    app.ui.export.renaming = None;
    match act {
        ListAction::Select(name) => app.ui.export.preset = name,
        ListAction::New => {
            save_new(app, &ExportPreset { name: tr("New template").into(), maps: Vec::new() });
        }
        ListAction::Duplicate => {
            if let Some(p) = selected(app) {
                let name = trf("{name} copy", &[("name", &p.name)]).chars().take(MAX_PRESET_NAME).collect();
                save_new(app, &ExportPreset { name, ..p });
            }
        }
        ListAction::StartRename(name) => {
            app.ui.export.preset = name.clone();
            app.ui.export.renaming = Some(name);
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique(RENAME_ID)));
        }
        ListAction::Rename(name) => rename(app, name.trim()),
        ListAction::Remove => {
            let name = app.ui.export.preset.clone();
            if app.run("export.preset.remove", json!({ "name": name })).is_some() {
                app.ui.export.preset = crate::state::ExportUi::default().preset;
                app.toast(trf("Deleted template \"{name}\"", &[("name", &name)]), false);
            }
        }
        ListAction::Import => import_file(app, ctx),
        ListAction::Export => export_file(app),
    }
}

/// Save `preset` as a new user preset (renamed when its name is taken) and select it.
fn save_new(app: &mut TextureCraftApp, preset: &ExportPreset) -> Option<String> {
    let v = serde_json::to_value(preset).ok()?;
    let r = app.run("export.preset.save", json!({ "preset": v, "unique": true }))?;
    let name = r.get("name").and_then(Value::as_str)?.to_string();
    app.ui.export.preset = name.clone();
    Some(name)
}

fn rename(app: &mut TextureCraftApp, name: &str) {
    let Some(mut p) = app.ui.export.draft.clone() else { return };
    if name.is_empty() || name == p.name {
        return;
    }
    let old = std::mem::replace(&mut p.name, name.to_string());
    let Ok(v) = serde_json::to_value(&p) else { return };
    if let Some(r) = app.run("export.preset.save", json!({ "preset": v, "rename": old }))
        && let Some(n) = r.get("name").and_then(Value::as_str)
    {
        app.ui.export.preset = n.to_string();
    }
}

/// Add an imported preset (renamed when its name is taken) and select it.
pub fn import(app: &mut TextureCraftApp, preset: &ExportPreset) {
    if let Some(name) = save_new(app, preset) {
        app.toast(trf("Imported template \"{name}\"", &[("name", &name)]), false);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import_file(app: &mut TextureCraftApp, _ctx: &egui::Context) {
    use crate::file_dialogs::{Bound, FileDialog};
    app.ask_file(FileDialog::open().filter(tr("Export template"), &["json"]), Bound::App, |app, paths| {
        let Some(path) = paths.first() else { return };
        match texturecraft_engine::formats::load_preset(path) {
            Ok(p) => import(app, &p),
            Err(e) => app.toast(trf("Couldn't import {name}: {error}", &[("name", &path.display()), ("error", &e)]), true),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn import_file(_app: &mut TextureCraftApp, ctx: &egui::Context) {
    crate::web_files::import_preset(ctx);
}

fn export_file(app: &mut TextureCraftApp) {
    let Some(p) = selected(app) else { return };
    let name = format!("{}.json", sanitize_file_stem(&p.name));
    let json = match texturecraft_engine::formats::preset_to_json(&p) {
        Ok(json) => json,
        Err(e) => {
            export_failed(app, &e);
            return;
        }
    };
    write_json(app, p.name, name, json);
}

fn export_failed(app: &mut TextureCraftApp, error: &dyn std::fmt::Display) {
    app.toast(trf("Couldn't export the template: {error}", &[("error", error)]), true);
}

fn exported(app: &mut TextureCraftApp, name: &str, to: &str) {
    app.toast(trf("Exported template \"{name}\" to {path}", &[("name", &name), ("path", &to)]), false);
}

/// Asks where to save `json` with the native dialog.
#[cfg(not(target_arch = "wasm32"))]
fn write_json(app: &mut TextureCraftApp, preset: String, name: String, json: String) {
    use crate::file_dialogs::{Bound, FileDialog};
    app.ask_file(FileDialog::save().filter(tr("Export template"), &["json"]).file_name(name), Bound::App, move |app, paths| {
        let Some(path) = paths.first() else { return };
        match std::fs::write(path, &json) {
            Ok(()) => exported(app, &preset, &path.display().to_string()),
            Err(e) => export_failed(app, &e),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn write_json(app: &mut TextureCraftApp, preset: String, name: String, json: String) {
    match crate::web_files::download(&name, json.as_bytes()) {
        Ok(()) => exported(app, &preset, &name),
        Err(e) => export_failed(app, &e),
    }
}

/// Read the user's templates: the library folder on the desktop, the app storage on the web.
pub fn load_library(app: &mut TextureCraftApp, storage: Option<&dyn eframe::Storage>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = storage;
        let skipped = app.session.load_export_presets(crate::user_dir("export-presets"));
        app.session.log.extend(skipped.into_iter().map(|w| format!("Export template skipped: {w}")));
    }
    #[cfg(target_arch = "wasm32")]
    for p in storage.and_then(|s| eframe::get_value::<Vec<ExportPreset>>(s, STORAGE_KEY)).unwrap_or_default() {
        if let Ok(v) = serde_json::to_value(&p) {
            app.run("export.preset.save", json!({ "preset": v, "unique": true }));
        }
    }
}

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "export_presets";

/// Keep the user's templates in the app storage (the web has no library folder).
pub fn save_library(app: &TextureCraftApp, storage: &mut dyn eframe::Storage) {
    #[cfg(target_arch = "wasm32")]
    eframe::set_value(storage, STORAGE_KEY, &app.session.export_presets.list);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (app, storage);
}

/// The selected template's output maps (editable for user templates).
fn outputs(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    let (mut preset, editable) = match (&app.ui.export.draft, selected(app)) {
        (Some(d), _) => (d.clone(), true),
        (None, Some(b)) => (b, false),
        (None, None) => {
            ui.label(RichText::new(tr("Select an output template")).color(tk.text_dim));
            return;
        }
    };
    let preview = preview_names(app, &preset);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&preset.name).strong().color(tk.text_strong));
        if !editable {
            ui.label(RichText::new(ic::LOCK_SIMPLE).color(tk.text_dim)).on_hover_text(tr("Built-in templates are read-only: duplicate one to edit it"));
        }
    });
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Add output")).color(tk.text_dim));
            for l in Layout::ALL {
                if ui.small_button(format!("{} {}", ic::PLUS, tr(l.label()))).clicked() {
                    action = Some(MapAction::Add(l));
                }
            }
        });
    });
    if let Err(e) = &preview {
        ui.label(RichText::new(trf("Not saved: {error}", &[("error", e)])).color(tk.danger).size(11.0));
    }
    let mut changed = false;
    let n = preset.maps.len();
    egui::ScrollArea::vertical().id_salt("exp_outputs").auto_shrink([false; 2]).show(ui, |ui| {
        if n == 0 {
            ui.add_space(8.0);
            ui.label(RichText::new(tr("No outputs yet: add one above, then drag input maps onto its channels.")).color(tk.text_dim));
        }
        ui.add_enabled_ui(editable, |ui| {
            for (mi, map) in preset.maps.iter_mut().enumerate() {
                let file = preview.as_ref().ok().and_then(|v| v.get(mi).cloned().flatten());
                changed |= output_card(ui, mi, n, map, file.as_deref(), &mut action);
                ui.add_space(4.0);
            }
        });
    });
    if !editable {
        return;
    }
    match action {
        Some(MapAction::Add(l)) => preset.maps.push(new_map(l)),
        Some(MapAction::Up(i)) if i > 0 && i < n => preset.maps.swap(i, i - 1),
        Some(MapAction::Down(i)) if i + 1 < n => preset.maps.swap(i, i + 1),
        Some(MapAction::Remove(i)) if i < n => {
            preset.maps.remove(i);
        }
        _ if !changed => return,
        _ => {}
    }
    commit(app, preset);
}

/// Keep the edited draft; save it when valid.
fn commit(app: &mut TextureCraftApp, preset: ExportPreset) {
    if preset.validate().is_ok()
        && let Ok(v) = serde_json::to_value(&preset)
    {
        app.run("export.preset.save", json!({ "preset": v }));
    }
    app.ui.export.draft = Some(preset);
}

/// The file each map produces for the current texture set with the dialog's settings (None:
/// the set has none of the map's sources), via `export.plan`.
fn preview_names(app: &mut TextureCraftApp, preset: &ExportPreset) -> Result<Vec<Option<String>>, String> {
    let mut p = crate::windows::export_params(app);
    p["preset"] = serde_json::to_value(preset).map_err(|e| e.to_string())?;
    p["ts"] = json!(app.session.ts());
    let plan = texturecraft_engine::execute(&mut app.session, "export.plan", &p).map_err(|e| e.to_string())?;
    let mut names = vec![None; preset.maps.len()];
    for f in plan.as_array().into_iter().flatten() {
        let map = f.get("map").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
        if let (Some(slot), Some(file)) = (map.and_then(|i| names.get_mut(i)), f.get("file").and_then(Value::as_str)) {
            *slot = Some(file.to_string());
        }
    }
    Ok(names)
}

/// One output file: name template, format, bit depth, layout and its channel sources.
fn output_card(ui: &mut Ui, mi: usize, n: usize, map: &mut MapSpec, file: Option<&str>, action: &mut Option<MapAction>) -> bool {
    let tk = Tokens::get(ui.ctx());
    let mut changed = false;
    egui::Frame::new().fill(tk.row).corner_radius(tk.radius).inner_margin(6.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let width = ui.available_width() - 4.0 * (ICON + ui.spacing().item_spacing.x);
            changed |= ui
                .add(egui::TextEdit::singleline(&mut map.file).font(egui::TextStyle::Monospace).desired_width(width))
                .on_hover_text(tr("File name without extension; variables insert names"))
                .changed();
            ui.menu_button(RichText::new(ic::CURRENCY_DOLLAR).size(13.0), |ui| {
                for (token, what) in FILE_TOKENS {
                    if ui.button(RichText::new(token).monospace()).on_hover_text(what).clicked() {
                        map.file.push_str(token);
                        changed = true;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(tr("Insert a variable"));
            if ui.add_enabled_ui(mi > 0, |ui| icon_button(ui, ic::ARROW_UP, tr("Move up"), false, ICON)).inner.clicked() {
                *action = Some(MapAction::Up(mi));
            }
            if ui.add_enabled_ui(mi + 1 < n, |ui| icon_button(ui, ic::ARROW_DOWN, tr("Move down"), false, ICON)).inner.clicked() {
                *action = Some(MapAction::Down(mi));
            }
            if icon_button(ui, ic::TRASH, tr("Remove this output"), false, ICON).clicked() {
                *action = Some(MapAction::Remove(mi));
            }
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("exp_map_fmt", mi)).width(56.0).selected_text(map.format.extension()).show_ui(ui, |ui| {
                for f in FileFormat::ALL {
                    if ui.selectable_value(&mut map.format, f, f.extension()).changed() {
                        map.bit_depth = f.depth(map.bit_depth);
                        changed = true;
                    }
                }
            });
            egui::ComboBox::from_id_salt(("exp_map_bits", mi)).width(64.0).selected_text(trf("{n} bits", &[("n", &map.bit_depth)])).show_ui(ui, |ui| {
                for &d in map.format.bit_depths() {
                    changed |= ui.selectable_value(&mut map.bit_depth, d, trf("{n} bits", &[("n", &d)])).changed();
                }
            });
            let layout = Layout::of(map);
            egui::ComboBox::from_id_salt(("exp_map_layout", mi)).width(72.0).selected_text(tr(layout.label())).show_ui(ui, |ui| {
                for l in Layout::ALL {
                    if ui.selectable_label(l == layout, tr(l.label())).clicked() && l != layout {
                        set_layout(map, l);
                        changed = true;
                    }
                }
            });
            let (text, color) = match file {
                Some(f) => (format!("{} {f}", ic::ARROW_RIGHT), tk.text_dim),
                None => (tr("Not exported: no source in this texture set").to_string(), tk.text_dim),
            };
            ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).truncate());
        });
        changed |= channel_slots(ui, mi, map);
    });
    changed
}

/// A source picker per output channel that also takes input maps dropped from the palette.
fn channel_slots(ui: &mut Ui, mi: usize, map: &mut MapSpec) -> bool {
    let tk = Tokens::get(ui.ctx());
    let mut changed = false;
    let mut dropped = None;
    for (k, (dest, src)) in map.channels.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let dest_name = match *dest {
                Dest::L => tr("Gray"),
                Dest::A => "A",
                d => RGB.get(slot_component(d) as usize).copied().unwrap_or("R"),
            };
            ui.add_sized(vec2(30.0, 18.0), egui::Label::new(RichText::new(dest_name).size(11.0).color(tk.text_dim)));
            let combo = egui::ComboBox::from_id_salt(("exp_slot", mi, k)).width(170.0).selected_text(tr(source_label(src))).show_ui(ui, |ui| {
                for (group, inputs) in PALETTE.iter() {
                    ui.label(caption(tr(group)));
                    for (label, s) in inputs {
                        if ui.selectable_label(same_source(src, s), tr(label)).clicked() {
                            *src = with_component(s, slot_component(*dest));
                            changed = true;
                        }
                    }
                }
            });
            let r = &combo.response;
            if r.dnd_hover_payload::<SourceRef>().is_some() {
                ui.painter().rect_stroke(r.rect.expand(2.0), 3.0, egui::Stroke::new(1.5, tk.accent), egui::StrokeKind::Outside);
            }
            if let Some(input) = r.dnd_release_payload::<SourceRef>() {
                dropped = Some((*dest, (*input).clone()));
            }
            let three = src.components() == 3;
            match src {
                SourceRef::Constant(v) => {
                    changed |= ui.add(egui::DragValue::new(v).range(0.0..=1.0).speed(0.005).fixed_decimals(2)).changed();
                }
                SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } if three => {
                    let shown = RGB.get(*component as usize).copied().unwrap_or("R");
                    egui::ComboBox::from_id_salt(("exp_slot_c", mi, k)).width(36.0).selected_text(shown).show_ui(ui, |ui| {
                        for (i, l) in (0u8..).zip(RGB) {
                            changed |= ui.selectable_value(component, i, l).changed();
                        }
                    });
                }
                _ => {}
            }
        });
    }
    if let Some((dest, input)) = dropped {
        drop_input(map, dest, &input);
        changed = true;
    }
    changed
}

/// Input maps to drag onto output channels.
fn input_maps(ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Input maps")));
    ui.label(RichText::new(tr("Drag onto an output channel")).size(11.0).color(tk.text_dim));
    egui::ScrollArea::vertical().id_salt("exp_inputs").auto_shrink([false; 2]).show(ui, |ui| {
        for (gi, (group, inputs)) in PALETTE.iter().enumerate() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr(group)).strong().size(11.0));
            ui.horizontal_wrapped(|ui| {
                for (k, (label, s)) in inputs.iter().enumerate() {
                    let label = tr(label);
                    // Not `dnd_drag_source`: its scope doesn't wrap inside `horizontal_wrapped`.
                    let r = chip(ui, label, false).interact(egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
                    r.dnd_set_drag_payload(s.clone());
                    if r.dragged()
                        && let Some(at) = r.interact_pointer_pos()
                    {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        egui::Area::new(egui::Id::unique(("exp_input_drag", gi, k)))
                            .order(egui::Order::Tooltip)
                            .fixed_pos(at + vec2(8.0, 8.0))
                            .show(ui.ctx(), |ui| chip(ui, label, true));
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(channel: Channel, component: u8) -> SourceRef {
        SourceRef::Channel { channel, component }
    }

    #[test]
    fn layouts_keep_sources_and_drops_fill_rgb() {
        let mut map = new_map(Layout::Rgb);
        drop_input(&mut map, Dest::G, &ch(Channel::BaseColor, 0));
        assert_eq!(map.channels, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]);
        drop_input(&mut map, Dest::B, &ch(Channel::Metallic, 0));
        assert_eq!(map.channels.get(2), Some(&(Dest::B, ch(Channel::Metallic, 0))), "a gray input fills only its channel");
        set_layout(&mut map, Layout::Rgba);
        assert_eq!(Layout::of(&map), Layout::Rgba);
        assert_eq!(map.channels.get(3), Some(&(Dest::A, SourceRef::Constant(1.0))));
        set_layout(&mut map, Layout::GrayAlpha);
        assert_eq!(map.channels, vec![(Dest::L, ch(Channel::BaseColor, 0)), (Dest::A, SourceRef::Constant(1.0))]);
        set_layout(&mut map, Layout::Rgb);
        assert_eq!(map.channels.get(1), Some(&(Dest::G, ch(Channel::BaseColor, 1))), "gray to RGB spreads a color source");
        for l in Layout::ALL {
            let m = new_map(l);
            assert_eq!(Layout::of(&m), l);
            ExportPreset { name: "t".into(), maps: vec![m] }.validate().unwrap();
        }
        assert!(same_source(&ch(Channel::Normal, 2), &ch(Channel::Normal, 0)));
        assert!(!same_source(&SourceRef::Constant(0.0), &SourceRef::Constant(1.0)));
    }
}
