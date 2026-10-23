#![cfg(feature = "native-ui")]

use eframe::egui::{
    Color32, ColorImage, Context, Event, Key, Modifiers, RawInput, Rect, Theme, UserData,
    ViewportId, pos2, vec2,
};
use gamestrike::native_capture::isolate_capture_input;
use std::sync::Arc;

fn pointer_and_key() -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1250.0, 800.0))),
        focused: true,
        system_theme: Some(Theme::Light),
        events: vec![
            Event::PointerMoved(pos2(350.0, 200.0)),
            Event::Key {
                key: Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn capture_has_no_hover_or_keyboard_authority_but_keeps_real_readback_and_viewport() {
    let ctx = Context::default();
    let mut input = pointer_and_key();
    let viewport = input.screen_rect;
    let image = Arc::new(ColorImage::filled([2, 2], Color32::BLACK));
    input.events.push(Event::Screenshot {
        viewport_id: ViewportId::ROOT,
        user_data: UserData::default(),
        image: image.clone(),
    });
    isolate_capture_input(&mut input, true);
    assert_eq!(input.screen_rect, viewport);
    assert!(input.focused);
    assert_eq!(input.system_theme, Some(Theme::Light));
    assert!(input.events.iter().any(|event| {
        matches!(event, Event::Screenshot { image: captured, .. } if Arc::ptr_eq(captured, &image))
    }));
    ctx.begin_pass(input);
    assert!(ctx.pointer_hover_pos().is_none());
    assert!(!ctx.input(|input| input.key_pressed(Key::ArrowDown)));
    ctx.end_pass().drop_without_applying_deltas();
}

#[test]
fn normal_interactive_input_is_preserved() {
    let ctx = Context::default();
    let mut input = pointer_and_key();
    isolate_capture_input(&mut input, false);
    ctx.begin_pass(input);
    assert_eq!(ctx.pointer_hover_pos(), Some(pos2(350.0, 200.0)));
    assert!(ctx.input(|input| input.key_pressed(Key::ArrowDown)));
    ctx.end_pass().drop_without_applying_deltas();
}
