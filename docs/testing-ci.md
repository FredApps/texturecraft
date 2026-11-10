# Integration tests and CI

The engine and UI integration tests share one `it` executable per crate. This
avoids compiling and linking the same crate dependencies into a separate
executable for every feature. Feature modules keep their own helpers and tests.
When adding a file under `tests/it/`, register it with `mod name;` in that
folder's `main.rs` so Cargo runs it.

Run a feature module with a test-name filter, for example:

```sh
cargo test -p texturecraft-engine --test it bake::
```

The standalone engine `session` target and UI `ui_snapshots` target remain
available for the commands in `AGENTS.md`. Existing ignore attributes are
unchanged. Tests in the shared executable can run concurrently across modules;
keep temporary paths unique and avoid changing process-wide state.

CI continues to use the shared gate workflow and `cargo xtask ci` on Ubuntu and
Windows: formatting, Clippy, workspace tests (including doctests), layering and
isolated per-package WebAssembly checks. The caller disables the shared
workflow's optional tool installation because this gate uses `cargo test` and
does not use nextest.
