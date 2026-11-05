# TextureCraft roadmap

TextureCraft aims for 1:1 parity with Adobe Substance 3D Painter's texturing workflow, with better
performance, a smaller footprint, every feature drivable by agents (MCP, CLI, control channel), and
builds for Windows, macOS, Linux, FreeBSD and the web.

## Honest parity estimate (2026-11-04)

Weighted by how much of a professional's daily work each area carries. "Done" counts what works
end to end today (engine + UI + automation + tests), not what is planned.

| Area | Weight | Done | What works | Main gaps |
|---|---:|---:|---|---|
| Project and files | 8 | 95% | New project from glTF/GLB, FBX (binary and ASCII, 6.x and 7.x), COLLADA, **USD (usda, usdc, usdz)**, **Alembic (Ogawa)**, OBJ, PLY, STL or built-in meshes (auto-unwrap when UVs are missing; a `.gltf`'s buffers and images and a loose USD layer's textures are read from its folder and kept in the project), **project templates** (built-in and saved from a project), `.tcraft` open/save/save as/save a copy, autosave, **asks to save unsaved changes** before quitting or replacing the project (in the browser too, and before the tab closes), recent files, drag and drop, reimport mesh **with strokes preserved on the surface**, **MaterialX import** (`file.importMaterialX`: a `.mtlx` material with `open_pbr_surface`, `standard_surface` or `gltf_pbr` becomes one fill layer with its textures and values), image resources, remove unused resources, textured OBJ (MTL), GLB and **USD** mesh export, browser open/save/download | HDF5 Alembic; Alembic animation (first sample only) |
| Texture sets | 6 | 100% | One set per material, **UDIM / UV tiles** (auto-detected or switched on later with resampling, **a resolution per tile** whose filters and height slopes keep their size in UV space, one texture per tile at its own size, `$udim` export), list with visibility (show or hide all, invert, **focus** on one set), sizes 128–8192 with resampling, add/remove channels (19 channel types incl. coat, sheen, anisotropy and scattering), height strength, normal format | — |
| Layer stack | 16 | 98% | Paint, fill, folder and **instance** layers; 32 per-channel blend modes and opacities; white/black/painted masks with **mask with height**; generator, fill, levels (with a histogram of its input), invert/threshold, color selection, compare, blur, sharpen and HSL effects on masks and layer content; anchor points; effects **dragged to reorder**, duplicated, and **copied, cut and pasted** across layers, masks and sets (Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D and Delete act on a selected effect); smart masks; mask view (Alt+click); Alt+click a material channel to keep it alone; duplicate, group, drag to reorder, rename, color tags, copy/cut/paste (also as instance) across sets, flatten, instantiate across sets, save layers as smart materials; **30 library filters** (adjustments incl. histogram scan, range and shift, blur/sharpen/high pass, bevel, glow, edge detect, warps, mosaic, baked lighting, material finishes); **geometry masks** (a layer on chosen mesh parts) | A few niche filters of the reference app's catalog |
| Painting tools | 14 | 91% | Paint, eraser, projection with image stencils, polygon fill, **path** and **lasso fill**, smudge, clone, color picker, quick mask, Shift+click lines with preview, 2D view painting; size, flow, stroke opacity, spacing, hardness, angle, follow path, jitters, alignment, angle fade, backface culling, lazy mouse, **Ctrl+drag in a view for size, hardness, flow and angle**, X/Y/Z mirror and **radial symmetry**, pen pressure in the 3D and 2D views (Windows Ink, and touch pens in the browser), procedural and image alphas, **text** in any installed font (as an alpha or a decal), **particle brush** | Pen tilt, and pen pressure on Linux, macOS and desktop browsers (not exposed by the windowing layer) |
| Materials and assets | 8 | 92% | **MaterialCraft materials** (`.mcmat`, sent to the shelf from MaterialCraft) as fill layers whose graph inputs and presets stay editable, re-cooked at the set's resolution with its baked mesh maps; 63 procedural materials (incl. car paint, velvet, brushed steel, skin, wax, ice, snow, bark), 21 smart materials, 24 smart masks, **67 procedural brush alphas**, **18 brush presets** plus user presets, 23 procedurals, 6 environments plus imported HDR/EXR panoramas, image resources, a user smart material library, a tabbed shelf with search and **drag and drop** onto layers and the viewport | A smaller bundled library than the reference app's |
| Baking | 10 | 90% | All ten mesh-map bakers; low-poly and high-to-low baking with CPU/GPU tracing; mesh-name matching, backface filtering, custom and distance-based cages, frontal/rear distances and average normals; undoable 2D cage/skew painting saved per texture set; rectangular output, antialiasing, dilation/diffusion, ID sources, ray filters, AO attenuation/ground plane, curvature controls and map normalization; per-texture-set selection, progress/cancel and mesh-map import | Residual Painter map differences; reference validation for Height, Bent Normal and Opacity and across additional assets; 3D cage-editing overlay |
| Viewport and display | 12 | 95% | PBR with image-based lighting, 6 procedural environments or imported HDR/EXR panoramas with exposure, rotation, blur, opacity, ACES/linear tone mapping, HDR pipeline with bloom, vignette, sharpen and grain, key-light shadows, 4× MSAA plus idle temporal accumulation, the baked normal map under the painted normal, channel / mesh-map / mask / UV checker views, 3D and 2D wireframes of the mesh's own polygons, brush cursor, axis gizmos, subsurface scattering blur, **depth of field**, saved cameras and cameras from glTF, FBX, COLLADA and USD files, displacement (also in shadows and the path tracers), **path-traced Rendering mode** (F10, on the GPU or the CPU) with environment importance sampling and an edge-aware **denoiser**; picks a discrete GPU on multi-GPU machines (`--gpu`, `WGPU_ADAPTER_NAME`, `WGPU_POWER_PREF`), and display textures are made on demand so a mesh with many materials imports on a small GPU | Hardware ray tracing |
| Shader | 6 | 100% | Metallic/roughness and specular/glossiness PBR, **clear coat, sheen, anisotropy and subsurface scattering** (viewport and path tracer), height-to-normal, painted normals, parallax occlusion, emissive, opacity (dithered), AO intensity | — |
| Export | 8 | 92% | 10 presets (incl. Godot 4 and **OpenPBR** with every shading lobe) plus **user output templates** (per-channel packing, converted maps, JSON import/export), PNG/JPEG/TGA/TIFF/BMP/WEBP/EXR/HDR, 8/16/32-bit, padding modes, size override, OpenGL/DirectX normals, **colour space** of the colour maps (sRGB, Linear Rec.709, ACEScg, ACES2065-1; data maps never converted; the chosen space is recorded in the file as an ICC profile in PNG, JPEG, TIFF and WebP and as chromaticities in EXR), `$udim` tiles, **MaterialX** (`file.exportTextures` with `materialX`: a `.mtlx` OpenPBR material per set, `<UDIM>` aware), OBJ with MTL and maps, textured GLB (with clearcoat/sheen) and USD (with clear coat) mesh export, zip download in the browser | Sheen and anisotropy in USD (UsdPreviewSurface has no inputs for them; GLB and OpenPBR maps carry them) |
| UI and UX | 8 | 97% | Painter-style layout with **dockable, tabbed, floating panels**, menus, tool shelf, tool bar, asset shelf, texture set list, layer stack with context menu, properties, display/shader/history/log panels, export dialog, new project dialog, bake mode, rendering mode, settings (interface scale, autosave, viewport texture resolution, system title bar), **own title bar** with the app icon and window buttons (Windows, GNOME; integrated with the macOS traffic lights), saved UI layouts, a **History** panel that jumps to any step (back or forward), a **start window** (new, open, recent projects), **shortcut editor** with conflict detection, tooltips with shortcuts, a compact color picker (saturation/value square, hue strip, copyable RGB); all menu items live ([docs/parity.md](docs/parity.md)); **10 UI languages** with CJK and Arabic fonts ([docs/localization-parity.md](docs/localization-parity.md)); closed panels reopen where they were; frame-budgeted compositing and uploads keep 8K sets responsive | Native-speaker review of the translations |
| Platforms | 4 | 76% | Windows, macOS (universal, signed and notarized), Linux (AppImage, deb, rpm, tar.gz, Flatpak), FreeBSD and web release workflows | First real release run; web build polish |
| **Total** | **100** | **≈ 94%** | Weighted workflow-completion estimate | |

Baking’s 90% estimates workflow completion: all ten map types and exposed settings have implementation
paths, with the remaining work listed above. Pixel-level equivalence with Painter remains unverified.

Beyond the reference app: every command is available to agents through MCP, the CLI and the app's
control channel (MCP workspace tools cover every file action: projects, meshes, high polys and
the cage, resources, smart materials, textures and mesh export), and the whole engine runs headless.

## Remaining effort

Estimated in wall-clock hours of Claude Opus 5.5 agent work, with three or four agents working in
parallel on separate crates:

| Milestone | Scope | Estimate |
|---|---|---:|
| M1 Layer stack depth | Done | 0 h |
| M2 Tools | Pen tilt, and pen pressure on Linux, macOS and desktop browsers, once the windowing layer reports them | 0.5 h |
| M3 Assets | Done | 0 h |
| M4 Baking | Validate outputs across reference assets, refine remaining map differences and add 3D cage editing | TBD |
| M5 Viewport | Hardware ray tracing: wgpu's ray queries are experimental and need an `unsafe` opt-in, which the workspace forbids (owner decision) | 1.5 h |
| M6 Shader | Done | 0 h |
| M7 Formats | Done | 0 h |
| M8 UX | Native review of translations | 1 h |
| M9 Release | The twelve signing secrets and the Default runner group (org admin); first signed release run on every platform (v0.1.0); web deployment; Flathub submission | 3 h |
| **Total** | | **≈ 6 h + baking TBD** |

## Current focus

1. Release v0.1.0 (M9): CI (fmt, clippy, the full test suite, layering and wasm on Linux and
   Windows) is green on `main`, and the `release` environment and branch ruleset exist. Left: an
   org admin syncs the twelve signing secrets and checks the Default runner group
   ([docs/releasing.md](docs/releasing.md)), then the first release run on every platform.
2. Hardware ray tracing in Rendering mode (M5) waits on an owner decision: wgpu exposes ray
   queries only as an experimental feature behind an `unsafe` opt-in, and the workspace forbids
   `unsafe` (craftrules allows it only in an isolated, tested helper crate).
3. Pen tilt, and pen pressure on Linux, macOS and desktop browsers (M2), once the windowing
   layer reports them; Alembic files in the HDF5 container and Alembic animation.
