# AGENTS.md: guide for AI agents and contributors

TextureCraft is an open-source, native 3D texture painter: a clean-room implementation of the
workflow known from Adobe Substance 3D Painter, written in **Rust only** (no JavaScript or TypeScript). **No Tauri, Electron or webview
shells:** the desktop app is native egui/eframe on wgpu, and the web build is the same Rust compiled
to WebAssembly (trunk + wasm-bindgen). Never add Tauri (or any webview/JS UI framework) as a
dependency, build step or packaging target. The product name is always written **TextureCraft**
(`{Function}Craft` in PascalCase, like its siblings PhotoCraft, VectorCraft, FilmCraft, LightCraft,
PdfCraft, EffectCraft, DesignCraft) in user-facing text: UI, window titles, About, installers,
release names, docs prose. Machine names stay lowercase: crates (`texturecraft-*`), binaries, file
names, ids (`ai.storyteller.texturecraft`), the native project extension `.tcraft`.

Standards and learnings shared across the crafting apps live in `../craftrules` (checked out next
to this repo, or `storytold/craftrules`). Read its `AGENTS.md` at the start of a session, follow its
standards, and contribute reusable learnings back there. Shared code goes only in craft-libs
(`standards/shared-libraries.md`); otherwise copy, don't depend.

The goal is 1:1 parity with Substance 3D Painter's workflow (same panels, menus, shortcuts, layer
stack semantics and export conventions) with better performance, every feature drivable by agents,
and builds for Windows, macOS, Linux, FreeBSD and the web. Read this file first, then `docs/`.

## 1. Orientation (5 minutes)

| Read | Why |
|---|---|
| `docs/architecture.md` | Crate map, layers, the CPU texel engine + GPU viewport split, data flow |
| `docs/control-protocol.md` | The JSON control channel and MCP: how agents drive the running app |
| `ROADMAP.md` | Honest parity estimate, milestones, **current focus** |
| `docs/parity.md` | Generated list of Painter menu items: live or missing |
| `docs/releasing.md` | Versioning (`cargo xtask version`), the `release` branch, packaging |
| `ATTRIBUTION.md` | Every bundled asset with author, source, license |
| `crates/<name>/README.md` (where present) | Public API of that crate |

## 2. Workspace map

```text
crates/
  core texel geom            L0 document model + blend math; sparse COW tiles, padding, noise; meshes, BVH, UV G-buffer, camera
  compose bake               L1 CPU layer-stack compositor (the oracle) + display conversion; mesh map bakers
  paint formats              L2 brush engine (strokes, stamps, visibility, commit); mesh import, texture export, .tcraft
  engine render              L3 Session + command registry (every action is a command); wgpu viewport renderer
  automation                 L4 MCP server + headless JSON-lines RPC + bridge to the running app
  ui-egui                    L5 egui shell (thin: every action goes through engine commands)
apps/
  texturecraft               desktop app (eframe/wgpu), `--control <port>` loopback control server
  texturecraft-cli           headless CLI (info, run, commands, bake, export, render, mcp, serve)
  texturecraft-web           the same app in the browser (trunk + wasm-bindgen)
xtask/                       cargo xtask layers | wasm | ci | version | ico | icns | parity
```

**Layering is enforced** by `cargo xtask layers`: a crate may depend only on lower layers. Nothing
below `ui-egui` may use egui, eframe, winit or rfd; only `render`, `ui-egui` and the apps may use
wgpu. A new crate must be registered in `xtask/src/layers.rs`.

**The UI crate is swappable.** `texturecraft-ui-egui` holds no document logic: it calls engine
commands (`texturecraft_engine::execute`) and the session's stroke API. Another front end could
replace it without touching the engine.

## 3. Golden rules

### Never name the reference product (absolutely essential)

- The repository never names the commercial product TextureCraft is modeled on, nor its vendor,
  in code, comments, docs, commit messages, test names, file names or UI text. Say "the reference
  app", "the professional model-painter workflow" or describe the feature itself.
- Notes that name it may live only in the gitignored `plan/` folder or in this `AGENTS.md` file

### Assets, icons and images (absolutely essential)

- **Never use any icon, image, logo, texture, material, brush, environment, font or other asset
  from Adobe, Avid, Image-Line or Autodesk products**, and never copy files from any installed
  commercial application (proprietary formats such as `.sbsar`, `.spexp`, `.sppr` or `.spsm`, HDR
  environments, UI icons). Not as reference files in the repo either.
- Every image or icon must be **open source, public domain, Creative Commons (CC0/CC-BY), or
  original** (made by a contributor who has the right to license it). Prefer generating content in
  code (TextureCraft's environments, materials, alphas, meshes and thumbnails are all procedural).
- **Every bundled asset gets a row in [`ATTRIBUTION.md`](ATTRIBUTION.md)** (path, title, author,
  source, license) **in the same change that adds it**, plus its license file next to it when the
  license requires one. Assets compiled in through dependencies (icon fonts, default fonts) get a
  row too. No row, no merge.
- Demo images and README screenshots show TextureCraft rendering original or public-domain content
  only. Never personal photos.
- The ArtCraft logos in `docs/brand/` are not open source (`docs/brand/LICENSE-brand.txt`).
- Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts); never commit font
  files here.

### Clean room

TextureCraft is an independent, clean-room implementation of a professional texture-painting
workflow. Work from public documentation, general domain knowledge, public algorithms
(GGX/Smith/Schlick, split-sum IBL, MikkTSpace, jump-flood/BFS dilation, Möller–Trumbore, SAH BVH,
spherical harmonics, reoriented normal mapping, W3C blend modes) and public engine conventions
(glTF, Unreal, Unity packing). Never copy proprietary code, shaders, presets or file formats;
Painter file compatibility (`.spp`, `.spexp`, `.sbsar`) is a non-goal. Always describe TextureCraft
as a clean-room implementation, in code, docs and commits.

### Never crash (outranks feature work)

People trust TextureCraft with hours of painting. A malformed mesh or project, a bad command or MCP
param, an odd keystroke or a full disk must produce an error the user or agent can act on, never a
panic. The shared standard is `../craftrules/standards/never-crash.md`.

- Non-test code never panics: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or
  `unimplemented!`. Every production crate root carries
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.
- No `unsafe` (`unsafe_code = "forbid"` for the workspace).
- Input-derived numbers are hostile: `get()` instead of `[i]`, checked/saturating arithmetic, caps
  on allocations (mesh triangles, image sizes, zip entries), NaN guards.
- Every crash fix comes with a regression test. The engine's `panic_hunt` test runs every command
  with hostile params; keep it green when adding commands.

### Architecture rules

1. **Everything is a command.** New user-visible behaviour = a command in the engine
   (`crates/engine/src/*_cmds.rs`, registered in `commands.rs`) with id, label, menu path,
   shortcut, params doc, `enabled` and `run`, plus tests. The menus, shortcuts, CLI, control channel
   and MCP all dispatch commands by id. Menu items come from the catalog in
   `crates/ui-egui/src/menus.rs`; an item goes live when its id is implemented.
2. **Programmatic calls never open dialogs.** A command run through the control channel or MCP runs
   with defaults; only `ui.menu.invoke` behaves like a menu click.
3. **Inspect shows the agent's work.** `project.inspect` / `session.inspect` must reveal every
   layer, mask, effect, fill source and painted tile count an agent changed.
4. **CPU oracle, GPU display.** Layers live as sparse copy-on-write 64×64 tiles; compositing,
   painting, baking and export run on the CPU per dirty tile (rayon) and are fully testable; dirty
   tiles are uploaded to wgpu textures for the viewport. Any GPU acceleration must match the CPU
   compositor in parity tests.
5. **Undo is snapshots.** `Session::edit` clones the document (copy-on-write tiles make this cheap)
   and restores it on error, so every command is atomic.
6. **Never break wasm.** L0–L5 must `cargo check --target wasm32-unknown-unknown`
   (`cargo xtask wasm`). File-system code is `cfg(not(target_arch = "wasm32"))` with byte-based
   alternatives.
7. **Performance is a feature.** Never scan a full texture set per frame; work on dirty tiles.
   Benchmark heavy paths (bake, composite, export) in release and record numbers in the dev log.
8. **Cross-app formats are shared conventions.** MaterialCraft's `.mcmat` portable materials are
   read here with our own code (`formats::mcmat`): keep the manifest fields, usage ids and
   mesh-map identifiers (`BakedMap::id`) in step with MaterialCraft's
   `docs/control-protocol.md`, and never depend on MaterialCraft's crates; talk to it through
   `materialcraft-cli`.
9. **Verify UI changes visually.** Launch with `--control <port>` and call `ui.screenshot` (whole
   window) or `viewport.screenshot`; look at the PNG. Without a visible screen (a locked machine,
   CI), render the whole window offscreen:
   `TC_UI_SNAPSHOTS=<dir> cargo test -p texturecraft-ui-egui --release --test ui_snapshots -- --ignored`
   (add `CRAFT_FONTS_DIR=<craft-fonts checkout>` to see the release fonts).

## 4. Picking work

1. `ROADMAP.md` → **Current focus** and the parity table.
2. `docs/parity.md` → missing menu items, grouped by menu.
3. Run the app, take screenshots of TextureCraft and fix the biggest visible gaps in the workflow.
4. `log/devlog.md` (local) → the "Still open" bullets of recent entries.

## 5. Before you finish a task

```sh
cargo fmt --all
cargo test -p <crates you touched> --release
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm            # if you touched L0–L5
cargo test -p texturecraft-engine --test session panic_hunt   # if you added/changed commands
```

Then append a terse entry to `log/devlog.md` (what landed, numbers, what's still open). Commit
after every arc of work that builds and passes.

## 6. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target/agent-<name>`) and edit only the files you own.
  Shared files (`engine/src/commands.rs`, `engine/src/lib.rs`, `ui-egui/src/menus.rs`,
  `ui-egui/src/lib.rs`, the root `Cargo.toml`) get small, surgical edits; re-read before editing.
- Put new commands in a **new module** with its own `specs()` registered by one line.
- Write manifests atomically; one broken `Cargo.toml` breaks every agent's build.
- Delete your `target/agent-*` dir when done.

## 7. Where things are tracked

- `ROADMAP.md`: milestones, parity estimate and remaining effort.
- `docs/parity.md`: generated Painter menu coverage.
- `docs/releasing.md` and `../craftrules/release/playbook.md`: release builds and signing.
- `plan/` (local, gitignored): execution plan and notes.
- `log/` (local, gitignored): the dev log.
