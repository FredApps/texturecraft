# Crafted USDC acceptance fixture

`crafted-references.usdc` is synthetic test data, not an external product asset.

Generated from `scene::write::tests::crafted_crate_with_references_time_samples_and_inlined_values` in [craft-libs at 3dff4793](https://github.com/storytold/craft-libs/blob/3dff4793f1a5bd1f99a4fe3280d48f515158dff3/crates/usd/src/scene/write.rs). The generator's existing assertions passed in [this hosted run](https://github.com/FredApps/texturecraft/actions/runs/44688984933). [Committed generation wrapper](https://github.com/FredApps/texturecraft/blob/15a07f4fc845f2d4d6594dd07e8b34e8e77de456/.github/workflows/validate-pr270.yml). Generation added only a write of the completed byte buffer; no writer logic was changed.

The fixture restores the application-boundary assertions from TextureCraft's [pre-cutover test](https://github.com/storytold/texturecraft/blob/e099725db4425c0149eca46198acf5cc5575cced/crates/formats/src/mesh_export/usdc.rs). A class prim is referenced by a def prim with reference custom data, a double3 translation uses its first time sample, and an inlined diagonal matrix scales the single triangle. The class itself must not draw. Expected final bounds are (1, 0, 0) through (3, 2, 0).

Size: 1,170 bytes.
SHA-256: `f270b11c6bb21c3e5b9106b5af0697d4559cbdbe36c8be8539c40d72d01d1519`.

Source copyright: ArtCraft Team and the craft-libs contributors. Distributed under MIT; see `LICENSE-MIT` beside this fixture.
