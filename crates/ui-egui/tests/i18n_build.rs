//! Optional catalog embedding must work without network or application dependencies.
#[path = "../i18n_build.rs"]
#[allow(dead_code)] // The production build entry point is not invoked by these pure-input tests.
mod i18n_build;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("texturecraft-i18n-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(path.join("locales/es")).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn no_input_is_english_but_required_input_errors() {
    assert!(!i18n_build::generate(None, false).unwrap().contains("include_str!"));
    assert!(i18n_build::generate(None, true).is_err());
}

#[test]
fn invalid_and_relative_checkouts_are_rejected() {
    let fixture = Fixture::new();
    assert!(i18n_build::generate(Some(&fixture.0.join("missing")), false).is_err());
    assert!(i18n_build::generate(Some(std::path::Path::new("relative")), false).is_err());
}

#[test]
fn shared_only_locale_is_not_offered_and_required_needs_app_catalog() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("locales/es/common.tsv"), "\tSave\tGuardar\n").unwrap();
    assert!(!i18n_build::generate(Some(&fixture.0), false).unwrap().contains("include_str!"));
    assert!(i18n_build::generate(Some(&fixture.0), true).is_err());
}

#[test]
fn app_catalog_is_embedded_with_optional_shared_catalog() {
    let fixture = Fixture::new();
    let app = fixture.0.join("locales/es/texturecraft.tsv");
    std::fs::write(&app, "\tLayers\tCapas\n").unwrap();
    let source = i18n_build::generate(Some(&fixture.0), true).unwrap();
    assert!(source.contains("texturecraft.tsv"));
    assert!(source.contains("(\"es\", \"\","));
    let common = fixture.0.join("locales/es/common.tsv");
    std::fs::write(&common, "\tSave\tGuardar\n").unwrap();
    assert!(i18n_build::generate(Some(&fixture.0), true).unwrap().contains("common.tsv"));
}

#[test]
fn unreadable_catalog_fails_before_generating_rust_source() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("locales/es/texturecraft.tsv"), [0xff, 0xfe]).unwrap();
    assert!(i18n_build::generate(Some(&fixture.0), true).is_err());
}

#[test]
fn generated_paths_compile_and_embed_the_catalog_contents() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("locales/es/texturecraft.tsv"), "\tLayers\tCapas\n").unwrap();
    let generated = i18n_build::generate(Some(&fixture.0), true).unwrap();
    let program = format!(
        "extern crate self as craft_i18n;\npub type Embedded = [(&'static str, &'static str, &'static str)];\n{generated}\nfn main() {{ assert_eq!(CRAFT_I18N[0].2, \"\\tLayers\\tCapas\\n\"); }}"
    );
    let source = fixture.0.join("check.rs");
    let binary = fixture.0.join(format!("check{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&source, program).unwrap();
    assert!(std::process::Command::new("rustc").arg(&source).arg("-o").arg(&binary).status().unwrap().success());
    assert!(std::process::Command::new(binary).status().unwrap().success());
}
