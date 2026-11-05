# texturecraft-bake

Clean-room mesh-map baking from low-poly UVs and optional high-poly meshes. Persisted settings live in `texturecraft-core`; localized controls live in `texturecraft-ui-egui`. Types have individual files, with singular module folders.

`bake(&BakeRequest, &Progress)` runs the CPU oracle. `bake_on` accepts a `tracing::Tracer` such as the render crate's GPU baker, and retries on the CPU if that tracer fails. Both receive identical transfer rays and secondary-ray samples. Output is deterministic, floating point, and stored in rows from top to bottom. Cancellation is checked between bounded batches and before returning the result.

## Projection and common settings

- Independent output width and height, antialiasing, per-UV-tile dilation, and optional background diffusion
- High-to-low matching: Always by default, or By Mesh Name using the configured low/high suffixes
- Transfer-ray backface filtering, distance-based or custom-file cages, frontal/rear distances, and averaged normals
- Relative distances use the longest bounding-box extent; absolute distances use imported mesh units
- Painted cage values multiply the frontal distance; painted skew values blend toward mesh or face normals, with optional hard-edge protection

`BakePaintMap` contains pixel data only. Cage and skew settings remain in their respective model containers. The engine stores paint maps per texture set in document snapshots and project files. The shared 2D brush supports undo, redo, reset, and UV-layout changes.

## Map settings

| Map | Implemented behavior |
| --- | --- |
| Normal | Tangent-space high-poly normal, internally OpenGL; flat without a high-poly transfer |
| World Normal | World-space shading normal |
| ID | Vertex Color, Material Color, Mesh ID, Submesh ID, or File ID; Hue Shift, Random, and Grayscale generation |
| Ambient Occlusion | Independent ray count, distance interval, units, cone and distribution; backface and self-occlusion modes; None, Linear or Smooth attenuation; optional ground plane |
| Bent Normal | Independent occlusion settings and mean visible direction, encoded in world space |
| Thickness | Inward ray distances with self-occlusion filtering; raw, ray-distance or per-UV-tile min/max normalization |
| Curvature | Mesh-ray curvature with radius, ray count, self-intersection and automatic/manual tonemapping; normal-map curvature with detail strength and corresponding UV-edge seam blending |
| Position | All axes or a selected axis; scene or material bounds; box, uniform sphere-style or raw coordinates |
| Height | Signed high-poly displacement with low-mesh, min/max, ray-distance or manual normalization |
| Opacity | High-poly hit coverage multiplied by source vertex/material/texture alpha |

ID Material Color reads the material factor independently of textures and vertex colors. A source without material colors uses neutral gray (0.6). Generated IDs are deterministic. Normal maps are stored in OpenGL convention; display/export apply the project's normal format.

All dropdowns use enums. `BakedMap::id()` and `BakedMap::parse()` provide the shared MaterialCraft mesh-map identifiers. `bake.settings` accepts the complete nested model. No legacy Color or World Orientation map migration is provided.

## Validation and limits

Regression tests cover projection, matching, ray filters, normalization, rectangular maps, painting, persistence, and CPU/GPU agreement. The supplied 2048² high-to-low reference is evaluated separately, outside the repository, on UV-covered pixels. Numerical differences remain, especially in curvature and occlusion; exact Painter parity is not claimed. Painted cage control currently uses the 2D UV brush; it does not provide a 3D cage-editing overlay.

CPU benchmark: `cargo test -p texturecraft-bake --release -- --ignored --nocapture bench`.
GPU comparison: `cargo test -p texturecraft-render --release --lib -- --ignored --nocapture gpu_bake_speed`.
