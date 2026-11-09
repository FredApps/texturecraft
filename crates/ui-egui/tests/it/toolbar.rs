//! Toolbar layout and input coverage

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::{Session, Tool};
use texturecraft_ui_egui::state::{Mode, ViewLayout};
use texturecraft_ui_egui::{TextureCraftApp, theme, toolbar};

fn toolbar_harness(width: f32, tool: Tool) -> Harness<'static, TextureCraftApp> {
    let session = Session::new_project("Toolbar", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    let context = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = TextureCraftApp::new(&context, session);
    app.session.tool.tool = tool;
    app.session.tool.brush.lazy = 20.0;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 200.0))
        .with_context_setup(|context| {
            theme::install(context);
            theme::apply(context, theme::Brightness::default(), theme::Accent::default());
        })
        .build_ui_state(|ui, app| toolbar::paint_bar(app, ui), app);
    harness.run_steps(4);
    harness
}

fn size_label(harness: &Harness<'_, TextureCraftApp>) -> (egui::Pos2, egui::Rect) {
    harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Size" => Some((text.pos, shape.clip_rect)),
            _ => None,
        })
        .unwrap()
}

#[test]
fn narrow_toolbars_keep_scroll_arrows_and_mode_buttons_separate() {
    let mut harness = toolbar_harness(700.0, Tool::Path);
    for button_size in [20.0, 24.0, 28.0] {
        harness.ctx.all_styles_mut(|style| style.spacing.interact_size = egui::Vec2::splat(button_size));
        for width in [500.0, 450.0, 400.0, 350.0, 240.0, 200.0] {
            let width = width * button_size / 24.0;
            harness.set_size(egui::vec2(width, 200.0));
            harness.run_steps(4);
            let left = harness.get_by_label("Scroll left").rect();
            let right = harness.get_by_label("Scroll right").rect();
            let bake = harness.get_by_label("Bake Mesh Maps (F8)").rect();
            let (_, clip) = size_label(&harness);
            for button in [left, right, bake] {
                assert_eq!(button.size(), egui::Vec2::splat(button_size));
            }

            assert!(left.right() <= right.left(), "arrows overlap at width {width}: {left:?}, {right:?}");
            assert!(right.right() < bake.left(), "scrolling overlaps the mode buttons at width {width}");
            assert!(clip.left() >= left.right() && clip.right() <= right.left(), "content covers an arrow at width {width}: {clip:?}");
            assert!(harness.ctx.content_rect().contains_rect(left) && harness.ctx.content_rect().contains_rect(bake));
        }
    }
}

#[test]
fn arrows_and_wheel_scroll_without_changing_the_brush() {
    let mut harness = toolbar_harness(700.0, Tool::Path);
    let brush = harness.state().session.tool.brush.clone();
    let start = size_label(&harness).0.x;
    harness.get_by_label("Scroll right").click();
    harness.run_steps(30);
    let scrolled = size_label(&harness).0.x;
    assert!(scrolled < start - 50.0);

    harness.get_by_label("Scroll left").click();
    harness.run_steps(30);
    assert!((size_label(&harness).0.x - start).abs() < 1.0);

    let (_, clip) = size_label(&harness);
    harness.hover_at(clip.center());
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-100.0, 0.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(30);
    assert!(size_label(&harness).0.x < start - 50.0);
    assert_eq!(harness.state().session.tool.brush, brush);
}

#[test]
fn shorter_tools_and_wider_windows_remove_the_arrows() {
    let mut harness = toolbar_harness(700.0, Tool::Path);
    harness.get_by_label("Scroll right").click();
    harness.run_steps(30);
    harness.state_mut().session.tool.tool = Tool::PolygonFill;
    harness.state_mut().session.tool.fill_mode = texturecraft_engine::FillMode::Mesh;
    harness.run_steps(4);
    assert!(harness.query_by_label("Scroll right").is_none());
    let triangle = harness.get_by_label("Triangle fill").rect();
    harness.get_by_label("Triangle fill").click();
    harness.run_steps(2);
    assert!(harness.ctx.content_rect().contains_rect(triangle));
    assert_eq!(harness.state().session.tool.fill_mode, texturecraft_engine::FillMode::Triangle);

    harness.state_mut().session.tool.tool = Tool::Path;
    harness.set_size(egui::vec2(1600.0, 200.0));
    harness.run_steps(4);
    assert!(harness.query_by_label("Scroll right").is_none());
    let (position, clip) = size_label(&harness);
    assert!(clip.contains(position));
}

#[test]
fn compact_view_buttons_can_be_scrolled_into_view_and_clicked() {
    let mut harness = toolbar_harness(400.0, Tool::Path);
    for _ in 0..20 {
        let left = harness.get_by_label("Scroll left").rect();
        let right = harness.get_by_label("Scroll right").rect();
        let visible = harness.query_by_label("UV checker").is_some_and(|node| node.rect().left() > left.right() && node.rect().right() < right.left());
        if visible {
            harness.get_by_label("UV checker").click();
            harness.run_steps(3);
            assert_eq!(harness.state().session.view, texturecraft_engine::ViewMode::UvChecker);
            return;
        }

        harness.get_by_label("Scroll right").click();
        harness.run_steps(30);
    }

    panic!("the view buttons never became accessible");
}

#[test]
fn mode_buttons_keep_their_order_and_dispatch_commands() {
    let mut harness = toolbar_harness(700.0, Tool::Paint);
    for (label, mode) in [("Bake Mesh Maps (F8)", Mode::Baking), ("Rendering (path traced) (F10)", Mode::Rendering), ("Painting (F9)", Mode::Painting)] {
        let bake = harness.get_by_label("Bake Mesh Maps (F8)").rect();
        let paint = harness.get_by_label("Painting (F9)").rect();
        let render = harness.get_by_label("Rendering (path traced) (F10)").rect();
        assert!(bake.right() < paint.left() && paint.right() < render.left());
        harness.get_by_label(label).click();
        harness.run_steps(3);
        assert_eq!(harness.state().ui.mode, mode);
    }
}

#[test]
fn view_dropdowns_set_the_layout_and_projection() {
    let mut harness = toolbar_harness(1600.0, Tool::Paint);
    harness.get_by_label("3D/2D views (F1)").click();
    harness.run_steps(3);
    harness.get_by_label_contains("3D view only").click();
    harness.run_steps(3);
    assert_eq!(harness.state().ui.layout, ViewLayout::Only3D);
    assert!(harness.query_by_label_contains("2D view only").is_none(), "the layout menu stays open");
    assert!(harness.query_by_label("3D view only (F2)").is_some());

    harness.get_by_label("Perspective").click();
    harness.run_steps(3);
    harness.get_by_label("Perspective").click();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("Orthographic").is_none(), "a second click keeps the projection menu open");

    harness.get_by_label("Perspective").click();
    harness.run_steps(3);
    harness.get_by_label_contains("Orthographic").click();
    harness.run_steps(3);
    assert!(harness.state().session.camera.ortho);
    assert!(harness.query_by_label("Orthographic").is_some());
}

#[test]
#[ignore]
fn toolbar_frame_timing() {
    for width in [1600.0, 1000.0, 400.0] {
        let mut harness = toolbar_harness(width, Tool::Path);
        harness.run_steps(100);
        let mut timings = [0.0_f64; 5];
        for timing in &mut timings {
            let start = std::time::Instant::now();
            harness.run_steps(1000);
            *timing = start.elapsed().as_secs_f64() * 1000.0;
        }
        timings.sort_by(f64::total_cmp);
        eprintln!("Path + lazy mouse, width {width}: {:.2} us/frame median, batches {timings:?}", timings[2]);
    }
}

#[test]
#[ignore]
fn toolbar_window_snapshots() {
    let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else {
        return;
    };

    std::fs::create_dir_all(&directory).unwrap();
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).wgpu().build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().session.tool.tool = Tool::Path;
    harness.state_mut().session.tool.brush.lazy = 20.0;
    for (width, scale) in [(1600.0, 1.0), (960.0, 1.0), (960.0, 1.5)] {
        harness.set_size(egui::vec2(width, 960.0));
        harness.state_mut().ui.prefs.ui_scale = scale;
        harness.run_steps(6);
        harness.render().unwrap().save(directory.join(format!("toolbar-{width}-{scale}.png"))).unwrap();
    }
}
