//! Source-first native contracts. Execution has deliberately NOT been observed.
//! A removed clamp, early boot reveal, DPI-scaled hit target, or unclipped tile
//! would each break a distinct assertion below. These are not pixel-parity tests.
#![cfg(feature = "native-ui")]

use eframe::egui::{Rect, pos2, vec2};
use gamestrike::native_ui::{
    boot::{BOOT_END, BootPose},
    geometry::{Geometry, chamfer, contains_chamfer},
};

#[path = "../src/native_ui/cache_budget.rs"]
mod cache_budget;
#[path = "../src/native_ui/mesh_geometry.rs"]
mod mesh_geometry;
#[path = "../src/native_ui/navigation.rs"]
mod navigation;
mod geometry {
    pub use gamestrike::native_ui::geometry::contains_chamfer;
}
#[path = "../src/native_ui/activation.rs"]
mod activation;

#[test]
fn native_keyboard_click_uses_the_non_pointer_activation_path() {
    use eframe::egui::{Id, Sense, Ui, UiBuilder};
    let (ctx, input) = key_input(eframe::egui::Key::Enter);
    ctx.begin_pass(input);
    let id = Id::new("native.test.button");
    ctx.memory_mut(|memory| memory.request_focus(id));
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0, 40.0));
    let ui = Ui::new(
        ctx.clone(),
        Id::new("native.test.root"),
        UiBuilder::new().max_rect(rect),
    );
    let response = ui.interact(rect, id, Sense::click());
    assert!(activation::activated(&ui, &response, rect, 8.0, true));
    assert!(!activation::activated(&ui, &response, rect, 8.0, false));
    let _ = ctx.end_pass();
}

#[test]
fn real_pointer_clicks_in_a_chamfer_cutout_are_rejected() {
    use eframe::egui::{
        Context, Event, Id, Modifiers, PointerButton, RawInput, Sense, Ui, UiBuilder,
    };
    let ctx = Context::default();
    let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(120.0, 40.0));
    let point = rect.left_top() + vec2(1.0, 1.0);
    for (frame, events) in [
        vec![],
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        vec![Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    ]
    .into_iter()
    .enumerate()
    {
        ctx.begin_pass(RawInput {
            time: Some(frame as f64 * 0.1),
            events,
            ..Default::default()
        });
        let ui = Ui::new(
            ctx.clone(),
            Id::new("native.cutout.root"),
            UiBuilder::new().max_rect(rect),
        );
        let response = ui.interact(rect, Id::new("native.cutout.button"), Sense::click());
        if frame == 2 {
            assert!(response.clicked_by(PointerButton::Primary));
            assert!(!activation::activated(&ui, &response, rect, 8.0, true));
        }
        let _ = ctx.end_pass();
    }
}

#[test]
fn zero_and_mixed_zero_chamfers_triangulate_the_complete_plate_area() {
    let rect = Rect::from_min_size(pos2(17.0, 21.0), vec2(176.0, 236.0));
    for cuts in [
        [0.0; 4],
        [0.0, 12.0, 0.0, 12.0],
        [4.0, 8.0, 0.0, 4.0],
        [10.0; 4],
    ] {
        let polygon = mesh_geometry::clean_polygon(&chamfer(rect, cuts));
        let triangles = mesh_geometry::triangles(&polygon);
        assert_eq!(triangles.len(), polygon.len() - 2, "{cuts:?}");
        let area = |a: eframe::egui::Pos2, b: eframe::egui::Pos2, c: eframe::egui::Pos2| {
            ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
        };
        let covered: f32 = triangles
            .iter()
            .map(|&[a, b, c]| area(polygon[a], polygon[b], polygon[c]))
            .sum();
        let expected = rect.area() - cuts.iter().map(|cut| cut * cut * 0.5).sum::<f32>();
        assert!(
            (covered - expected).abs() < 0.05,
            "{cuts:?}: covered {covered}, expected {expected}"
        );
        assert!(polygon.windows(2).all(|pair| pair[0] != pair[1]));
    }
}

#[test]
fn resident_cover_budget_evicts_oldest_until_both_limits_fit() {
    use cache_budget::{Resident, eviction_indices};
    let entries = [
        Resident {
            bytes: 8,
            last_used: 3,
        },
        Resident {
            bytes: 4,
            last_used: 1,
        },
        Resident {
            bytes: 4,
            last_used: 2,
        },
        Resident {
            bytes: 4,
            last_used: 4,
        },
    ];
    assert_eq!(eviction_indices(&entries, 4, 20), Vec::<usize>::new());
    assert_eq!(eviction_indices(&entries, 3, 20), vec![1]);
    assert_eq!(eviction_indices(&entries, 4, 12), vec![1, 2]);
    assert_eq!(eviction_indices(&entries, 0, 0), vec![1, 2, 0, 3]);
}

fn key_input(key: eframe::egui::Key) -> (eframe::egui::Context, eframe::egui::RawInput) {
    use eframe::egui::{Event, Modifiers, RawInput};
    (
        eframe::egui::Context::default(),
        RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
    )
}

#[test]
fn one_sidebar_arrow_cannot_cascade_through_newly_focused_rows() {
    use navigation::{NavAction, take_nav_action};
    let (ctx, input) = key_input(eframe::egui::Key::ArrowDown);
    ctx.begin_pass(input);
    ctx.input_mut(|state| {
        assert_eq!(take_nav_action(state, 0), Some(NavAction::Select(1)));
        assert_eq!(take_nav_action(state, 1), None);
        assert_eq!(take_nav_action(state, 2), None);
    });
    let _ = ctx.end_pass();
}

#[test]
fn entering_games_consumes_right_arrow_before_grid_navigation() {
    use navigation::{NavAction, take_grid_action, take_nav_action};
    let (ctx, input) = key_input(eframe::egui::Key::ArrowRight);
    ctx.begin_pass(input);
    ctx.input_mut(|state| {
        assert_eq!(take_nav_action(state, 0), Some(NavAction::EnterGames));
        assert_eq!(take_grid_action(state, 0, 17, 6), None);
    });
    let _ = ctx.end_pass();
}

#[test]
fn grid_navigation_preserves_rows_and_consumes_each_key_once() {
    use navigation::{GridAction, take_grid_action};
    let (ctx, input) = key_input(eframe::egui::Key::ArrowDown);
    ctx.begin_pass(input);
    ctx.input_mut(|state| {
        assert_eq!(
            take_grid_action(state, 8, 17, 6),
            Some(GridAction::Select(14))
        );
        assert_eq!(take_grid_action(state, 14, 17, 6), None);
    });
    let _ = ctx.end_pass();
    let (ctx, input) = key_input(eframe::egui::Key::ArrowLeft);
    ctx.begin_pass(input);
    ctx.input_mut(|state| assert_eq!(take_grid_action(state, 6, 17, 6), Some(GridAction::Sidebar)));
    let _ = ctx.end_pass();
}

#[test]
fn boot_has_dark_ignition_reveal_and_exact_assembly_boundaries() {
    let dark = BootPose::at(0.0, false, false);
    assert_eq!(dark.ignite, 0.0);
    assert_eq!(dark.frame, 0.0);
    assert_eq!(dark.emblem, 0.0);
    assert!(!dark.settled());
    assert_eq!(BootPose::at(0.15, false, false).ignite, 0.0);
    assert!(BootPose::at(0.5, false, false).ignite > 0.9);
    let flash = BootPose::at(3.0, false, false);
    assert!(flash.flash > 0.7);
    assert_eq!(flash.emblem, 0.0);
    assert_eq!(flash.frame, 0.0);
    assert!(BootPose::at(3.45, false, false).emblem > 0.99);
    assert_eq!(BootPose::at(4.2, false, false).frame, 0.0);
    assert!(!BootPose::at(BOOT_END - 0.001, false, false).settled());
    let end = BootPose::at(BOOT_END, false, false);
    assert!(end.settled());
    assert_eq!(end.frame, 1.0);
    assert_eq!(end.content, 1.0);
    assert_eq!(end.fly, 1.0);
    assert_eq!(end.flash, 0.0);
}

#[test]
fn skip_and_reduced_motion_are_the_exact_settled_pose() {
    let end = BootPose::at(BOOT_END, false, false);
    for seconds in [-5.0, 0.0, 1.2, 3.0, 4.2, 5.499, 20.0, f32::NAN] {
        assert_eq!(BootPose::at(seconds, false, true), end);
        assert_eq!(BootPose::at(seconds, true, false), end);
        assert_eq!(BootPose::at(seconds, true, true), end);
    }
}

#[test]
fn time_is_total_deterministic_and_clamped() {
    let start = BootPose::at(0.0, false, false);
    for seconds in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY, -8.0] {
        assert_eq!(BootPose::at(seconds, false, false), start);
    }
    assert_eq!(
        BootPose::at(500.0, false, false),
        BootPose::at(BOOT_END, false, false)
    );
    for step in 0..=550 {
        let pose = BootPose::at(step as f32 * 0.01, false, false);
        assert_eq!(pose, BootPose::at(step as f32 * 0.01, false, false));
        for channel in [
            pose.ignite,
            pose.core,
            pose.flash,
            pose.emblem,
            pose.fly,
            pose.frame,
            pose.content,
            pose.net,
            pose.particles,
            pose.shockwave,
        ] {
            assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
        }
    }
}

#[test]
fn collection_regions_fit_viewports_and_do_not_overlap_navigation() {
    for size in [
        vec2(640.0, 360.0),
        vec2(960.0, 600.0),
        vec2(1250.0, 800.0),
        vec2(1672.0, 941.0),
        vec2(2560.0, 1440.0),
    ] {
        let viewport = Rect::from_min_size(pos2(13.0, 21.0), size);
        for dpi in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let g = Geometry::new(viewport, dpi);
            for region in [g.frame, g.sidebar, g.library, g.bottom] {
                assert!(region.is_finite() && region.is_positive());
                assert!(
                    viewport.contains_rect(region),
                    "{size:?} at {dpi}: {region:?}"
                );
            }
            assert!(g.sidebar.right() < g.library.left());
            assert!(g.library.bottom() < g.bottom.top());
            for (i, region) in g.navigation.iter().enumerate() {
                assert!(g.sidebar.contains_rect(*region));
                assert_eq!(g.navigation_at(region.center()), Some(i));
                assert_eq!(g.navigation_at(region.left_top()), None);
            }
            assert_eq!(g.navigation_at(g.library.center()), None);
            assert!(g.columns >= 1 && g.columns <= 12);
        }
    }
}

#[test]
fn native_collection_keeps_original_three_row_cover_density() {
    let reference = Geometry::new(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1672.0, 941.0)),
        1.0,
    );
    let capture = Geometry::new(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1250.0, 800.0)),
        1.0,
    );
    assert_eq!(reference.columns, 7);
    assert_eq!(capture.columns, 6);
}

#[test]
fn dpi_does_not_change_logical_layout_or_pointer_selection() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1250.0, 800.0));
    let reference = Geometry::new(rect, 1.0);
    for dpi in [0.75, 1.25, 1.5, 2.0, 3.0] {
        let scaled = Geometry::new(rect, dpi);
        assert_eq!(scaled.library, reference.library);
        assert_eq!(scaled.navigation, reference.navigation);
        for index in 0..28 {
            assert_eq!(
                scaled.card_rect(index, 0.0),
                reference.card_rect(index, 0.0)
            );
        }
    }
}

#[test]
fn card_hits_follow_chamfers_and_scrolled_clipping() {
    let g = Geometry::new(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1250.0, 800.0)),
        1.5,
    );
    let card = g.card_rect(0, 0.0);
    assert_eq!(g.card_at(card.center(), 40, 0.0), Some(0));
    assert_eq!(g.card_at(card.left_top(), 40, 0.0), None);
    assert_eq!(
        g.card_at(pos2(g.library.right() + 1.0, card.center().y), 40, 0.0),
        None
    );
    let offset = g.row_stride() * 2.0;
    let third_row = g.card_rect(g.columns * 2, offset);
    assert_eq!(
        g.card_at(third_row.center(), 40, offset),
        Some(g.columns * 2)
    );
    assert_eq!(
        g.card_at(
            pos2(third_row.center().x, g.library.top() - 1.0),
            40,
            offset
        ),
        None
    );
    let corner = chamfer(card, [g.card_cut(); 4]);
    assert_eq!(corner.len(), 8);
    assert!(contains_chamfer(card, g.card_cut(), card.center()));
    assert!(!contains_chamfer(card, g.card_cut(), card.left_top()));
}

#[test]
fn selected_card_can_be_scrolled_wholly_into_the_well() {
    let g = Geometry::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(960.0, 600.0)), 2.0);
    for index in [0, 7, 21, 63, 99] {
        let scroll = g.reveal_scroll(index, 100, 0.0);
        let card = g.card_rect(index, scroll);
        assert!(g.library.contains_rect(card));
        assert!(scroll >= 0.0 && scroll <= g.max_scroll(100));
    }
}

#[test]
fn invalid_geometry_inputs_produce_finite_safe_regions() {
    for dpi in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let g = Geometry::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(1.0, 1.0)), dpi);
        assert!(g.frame.is_finite());
        assert!(g.library.is_finite());
        assert!(g.max_scroll(usize::MAX).is_finite());
        assert_eq!(g.card_at(pos2(f32::NAN, 0.0), 3, f32::NAN), None);
    }
}
