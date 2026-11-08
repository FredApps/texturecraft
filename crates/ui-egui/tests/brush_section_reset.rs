//! Each brush section header has a reset icon that puts only that section's settings back to their defaults

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::paint::{Alignment, BrushSettings, Particles, Symmetry};
use texturecraft_ui_egui::TextureCraftApp;

/// The default window; Properties shows only the top of the brush there, so lower headers are scrolled to
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

/// Frames for a click to reach the brush settings and the panel
const SETTLE_FRAMES: usize = 3;

/// Puts one section's fields of the first brush back to the second's values
type SectionReset = fn(&mut BrushSettings, &BrushSettings);

/// Each brush section's title, a label shown only while it is open, whether it starts open, and its reset
const SECTIONS: [(&str, &str, bool, SectionReset); 7] = [
    ("Size", "Minimum size (%)", true, |brush, defaults| {
        (brush.size, brush.min_size, brush.screen_space) = (defaults.size, defaults.min_size, defaults.screen_space)
    }),
    ("Flow and opacity", "Stroke opacity", true, |brush, defaults| {
        (brush.flow, brush.opacity, brush.spacing, brush.hardness) = (defaults.flow, defaults.opacity, defaults.spacing, defaults.hardness)
    }),
    ("Angle", "Follow path", false, |brush, defaults| (brush.angle, brush.follow_path) = (defaults.angle, defaults.follow_path)),
    ("Jitters", "Size jitter", false, |brush, defaults| {
        (brush.jitter_size, brush.jitter_angle, brush.jitter_flow, brush.jitter_position) =
            (defaults.jitter_size, defaults.jitter_angle, defaults.jitter_flow, defaults.jitter_position)
    }),
    ("Alignment and culling", "Backface culling", false, |brush, defaults| {
        (brush.alignment, brush.backface_culling, brush.angle_fade, brush.depth) =
            (defaults.alignment, defaults.backface_culling, defaults.angle_fade, defaults.depth)
    }),
    ("Particles", "Gravity", false, |brush, defaults| brush.particles = defaults.particles),
    ("Lazy mouse and symmetry", "Lazy mouse distance", false, |brush, defaults| (brush.lazy, brush.symmetry) = (defaults.lazy, defaults.symmetry)),
];

/// A brush that differs from the defaults in every field a section shows, within the valid ranges
fn changed_brush() -> BrushSettings {
    BrushSettings {
        size: 0.1,
        min_size: 0.5,
        screen_space: true,
        flow: 0.5,
        opacity: 0.6,
        spacing: 0.5,
        hardness: 0.9,
        angle: 45.0,
        follow_path: true,
        jitter_size: 0.5,
        jitter_angle: 0.4,
        jitter_flow: 0.3,
        jitter_position: 1.0,
        alignment: Alignment::Camera,
        backface_culling: false,
        angle_fade: (30.0, 60.0),
        depth: 2.0,
        particles: Particles { enabled: true, count: 8, lifetime: 12, gravity: 0.5 },
        lazy: 50.0,
        symmetry: Symmetry { x: true, y: false, z: true, radial: 4, axis: 2 },
        ..BrushSettings::default()
    }
}

/// The app showing the brush in Properties, set to `brush`
fn app_with_brush(brush: BrushSettings) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().session.tool.brush = brush;
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// The reset icon on the header row titled `title`
fn reset_icon<'tree>(harness: &'tree Harness<'_, TextureCraftApp>, title: &str) -> egui_kittest::Node<'tree> {
    // "Size" and "Angle" also label sliders, so match the title on the icon's row
    let on_title_row = |icon: egui::Rect| harness.query_all_by_label(title).any(|label| label.rect().y_range().intersects(icon.y_range()));
    harness.query_all_by_label("Reset to defaults").find(|icon| on_title_row(icon.rect())).unwrap_or_else(|| panic!("no reset icon on the {title} header"))
}

/// Scrolls the header titled `title` into view and clicks its reset icon
fn click_reset(harness: &mut Harness<'_, TextureCraftApp>, title: &str) {
    reset_icon(harness, title).scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);

    reset_icon(harness, title).click();
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn each_reset_puts_back_only_its_own_section() {
    let defaults = BrushSettings::default();

    for (title, _, _, reset_section) in SECTIONS {
        let mut harness = app_with_brush(changed_brush());
        assert!(!reset_icon(&harness, title).accesskit_node().is_disabled(), "the {title} reset is enabled once its section changed");
        click_reset(&mut harness, title);

        let mut expected = changed_brush();
        reset_section(&mut expected, &defaults);
        assert_eq!(harness.state().session.tool.brush, expected, "resetting {title}");
    }
}

/// Scrolls the header titled `title` into view and clicks the header itself, folding or unfolding it
fn click_header(harness: &mut Harness<'_, TextureCraftApp>, title: &str) {
    reset_icon(harness, title).scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);

    let icon = reset_icon(harness, title).rect();
    harness.query_all_by_label(title).find(|label| label.rect().y_range().intersects(icon.y_range())).unwrap_or_else(|| panic!("no {title} header")).click();
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn clicking_the_reset_icon_does_not_fold_or_unfold_the_section() {
    for flip_first in [false, true] {
        let mut harness = app_with_brush(changed_brush());
        if flip_first {
            for (title, _, _, _) in SECTIONS {
                click_header(&mut harness, title);
            }
        }

        for (title, body_label, starts_open, _) in SECTIONS {
            let open = starts_open != flip_first;
            click_reset(&mut harness, title);
            assert_eq!(harness.query_by_label(body_label).is_some(), open, "{title} stays {}", if open { "open" } else { "closed" });
        }
    }
}

#[test]
fn at_the_defaults_the_reset_icons_are_inert() {
    let mut harness = app_with_brush(BrushSettings::default());

    for (title, body_label, starts_open, _) in SECTIONS {
        assert!(reset_icon(&harness, title).accesskit_node().is_disabled(), "the {title} reset is disabled at the defaults");

        click_reset(&mut harness, title);
        assert_eq!(harness.query_by_label(body_label).is_some(), starts_open, "the click doesn't reach the {title} header");
    }
}

#[test]
fn enter_on_the_focused_reset_icon_resets() {
    let mut harness = app_with_brush(changed_brush());

    reset_icon(&harness, "Size").focus();
    harness.run_steps(SETTLE_FRAMES);
    assert!(reset_icon(&harness, "Size").is_focused());

    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    let (brush, defaults) = (&harness.state().session.tool.brush, BrushSettings::default());
    assert_eq!((brush.size, brush.min_size, brush.screen_space), (defaults.size, defaults.min_size, defaults.screen_space));
}
