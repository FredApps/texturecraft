# Interface translations

TextureCraft uses the `craft-i18n` runtime from craft-libs and optional catalogs from
[craft-i18n](https://github.com/storytold/craft-i18n).

Set `CRAFT_I18N_DIR` to the absolute path of a craft-i18n checkout before building.
The build embeds `locales/<locale>/common.tsv` and `texturecraft.tsv`. Without catalog input,
the application uses English. `CRAFT_I18N_REQUIRED=1` makes missing/invalid input a build error.
The same catalogs are embedded for native and WebAssembly builds; no runtime files are needed.

Feature changes add English strings through `tr`, `n_` or `trf`. Translations belong in
craft-i18n PRs. A missing translation displays English and does not block a feature or release.
Run `cargo test -p texturecraft-ui-egui translation_coverage_report -- --nocapture` with
catalog input to inspect untranslated and unused/shared keys. Shared catalogs contain other
apps' strings, so unused shared keys are informational, not candidates for automatic deletion.

The initial migration is tracked in storytold/texturecraft#419. Catalog changes must merge
before the application cutover, and translations landing meanwhile need a final sync.
Saved `pt-BR` and `zh-Hans` preferences retain their original spelling and meaning.

## CI and release catalog revision

`craft-i18n.rev` pins the catalog commit. The shared craft-common-actions gate and release
workflows read this file, check out storytold/craft-i18n and set CRAFT_I18N_DIR together with
CRAFT_I18N_REQUIRED=1. No application workflow changes are needed.

The catalog commit must be available in storytold/craft-i18n before merging the app cutover.
After a squash or rebase merge of the companion catalog PR, update the pin to the resulting
upstream commit and rerun validation before merging TextureCraft.
