#![cfg(feature = "native-ui")]

use eframe::egui::{
    Context, Event, Id, Key, Modifiers, PointerButton, RawInput, Rect, Ui, UiBuilder, pos2, vec2,
};
use gamestrike::{
    application::AppCommand,
    music::{MusicCommand, MusicSnapshot, Playlist, Track},
    native_ui::music::MusicPanel,
};
use uuid::Uuid;

fn track(number: u128, title: &str) -> Track {
    Track {
        id: Uuid::from_u128(number),
        title: title.into(),
        path: format!("owned-fixture-{number}.wav").into(),
    }
}

fn snapshot() -> MusicSnapshot {
    MusicSnapshot {
        queue: vec![track(1, "Zebra"), track(2, "Moon"), track(3, "Amber")],
        playlists: vec![Playlist {
            id: Uuid::from_u128(20),
            name: "Saved order".into(),
            tracks: vec![track(3, "Amber"), track(1, "Zebra")],
        }],
        available: true,
        problem: None,
        revision: 1,
    }
}

struct Frame {
    commands: Vec<AppCommand>,
    labels: Vec<(String, f32)>,
}

#[derive(Default)]
struct Harness {
    ctx: Context,
    panel: MusicPanel,
    time: f64,
}

impl Harness {
    fn draw(&mut self, snapshot: &MusicSnapshot, events: Vec<Event>, enabled: bool) -> Frame {
        self.time += 0.1;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 560.0));
        self.ctx.begin_pass(RawInput {
            screen_rect: Some(rect),
            time: Some(self.time),
            focused: true,
            events,
            ..Default::default()
        });
        let mut ui = Ui::new(
            self.ctx.clone(),
            Id::new("native.music.test.root"),
            UiBuilder::new().max_rect(rect),
        );
        let commands = self.panel.show(&mut ui, rect, 1.0, snapshot, enabled);
        drop(ui);
        let output = self.ctx.end_pass();
        let mut labels = Vec::new();
        for clipped in &output.shapes {
            collect_labels(&clipped.shape, &mut labels);
        }
        output.drop_without_applying_deltas();
        Frame { commands, labels }
    }

    fn click(&mut self, snapshot: &MusicSnapshot, id: Id, enabled: bool) -> Frame {
        let point = self
            .ctx
            .read_response(id)
            .expect("visible Music control")
            .rect
            .center();
        let pressed = self.draw(
            snapshot,
            vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            enabled,
        );
        assert!(pressed.commands.is_empty());
        self.draw(
            snapshot,
            vec![Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            enabled,
        )
    }

    fn activate(&mut self, snapshot: &MusicSnapshot, id: &str, enabled: bool) -> Frame {
        self.ctx
            .memory_mut(|memory| memory.request_focus(Id::new(id)));
        let pressed = self.draw(snapshot, vec![key(Key::Enter, true)], enabled);
        let released = self.draw(snapshot, vec![key(Key::Enter, false)], enabled);
        assert!(released.commands.is_empty());
        pressed
    }
}

fn collect_labels(shape: &eframe::egui::Shape, labels: &mut Vec<(String, f32)>) {
    match shape {
        eframe::egui::Shape::Text(text) => {
            labels.push((text.galley.text().to_owned(), text.pos.y));
        }
        eframe::egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_labels(shape, labels);
            }
        }
        _ => {}
    }
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn keyboard_add_files_emits_one_host_intent_without_gpu_or_picker() {
    let mut harness = Harness::default();
    let snapshot = snapshot();
    harness.draw(&snapshot, vec![], true);
    let frame = harness.activate(&snapshot, "pwds.music.add", true);
    assert!(matches!(
        frame.commands.as_slice(),
        [AppCommand::ChooseMusicFiles]
    ));
    assert!(harness.draw(&snapshot, vec![], true).commands.is_empty());
}

#[test]
fn pointer_reorder_waits_for_the_committed_snapshot() {
    let mut harness = Harness::default();
    let mut snapshot = snapshot();
    harness.draw(&snapshot, vec![], true);
    let selected = snapshot.queue[1].id;
    assert!(
        harness
            .click(&snapshot, Id::new(("pwds.music.track", selected)), true)
            .commands
            .is_empty()
    );
    let frame = harness.click(&snapshot, Id::new("pwds.music.up"), true);
    assert!(matches!(
        frame.commands.as_slice(),
        [AppCommand::Music(MusicCommand::MoveTrack { id, up: true })] if *id == selected
    ));
    assert_eq!(snapshot.queue[0].title, "Zebra");
    assert_queue_labels(&frame, &["01  Zebra", "02  Moon", "03  Amber"]);
    assert!(harness.draw(&snapshot, vec![], true).commands.is_empty());

    snapshot.queue.swap(0, 1);
    snapshot.revision += 1;
    let committed = harness.draw(&snapshot, vec![], true);
    assert_queue_labels(&committed, &["01  Moon", "02  Zebra", "03  Amber"]);
}

#[test]
fn unavailable_or_unsettled_music_emits_no_intents() {
    for (available, enabled) in [(false, true), (true, false)] {
        let mut harness = Harness::default();
        let mut snapshot = snapshot();
        harness.draw(&snapshot, vec![], true);
        harness.click(
            &snapshot,
            Id::new(("pwds.music.track", snapshot.queue[1].id)),
            true,
        );
        harness.click(
            &snapshot,
            Id::new(("pwds.music.playlist", snapshot.playlists[0].id)),
            true,
        );
        harness
            .ctx
            .memory_mut(|memory| memory.request_focus(Id::new("pwds.music.name")));
        harness.draw(&snapshot, vec![Event::Text("Disabled draft".into())], true);
        snapshot.available = available;
        snapshot.problem = (!available).then(|| "Owned fixture store is unavailable".into());
        let frame = harness.draw(&snapshot, vec![], enabled);
        if !available {
            assert!(
                frame
                    .labels
                    .iter()
                    .any(|(label, _)| label == "Owned fixture store is unavailable")
            );
        }
        for id in [
            "pwds.music.add",
            "pwds.music.up",
            "pwds.music.down",
            "pwds.music.remove",
            "pwds.music.save",
            "pwds.music.load",
        ] {
            assert!(harness.activate(&snapshot, id, enabled).commands.is_empty());
        }
        assert!(
            harness
                .click(&snapshot, Id::new("pwds.music.add"), enabled)
                .commands
                .is_empty()
        );
    }
}

#[test]
fn rendered_queue_order_comes_from_the_snapshot_and_never_implies_playback() {
    let mut harness = Harness::default();
    let frame = harness.draw(&snapshot(), vec![], true);
    assert_queue_labels(&frame, &["01  Zebra", "02  Moon", "03  Amber"]);
    assert!(
        frame
            .labels
            .iter()
            .any(|(label, _)| label == "PLAYBACK NOT CONNECTED")
    );
    assert!(frame.commands.is_empty());
}

#[test]
fn playlist_name_and_selection_emit_save_and_load_intents_only() {
    let mut harness = Harness::default();
    let snapshot = snapshot();
    harness.draw(&snapshot, vec![], true);
    assert!(
        harness
            .activate(&snapshot, "pwds.music.save", true)
            .commands
            .is_empty()
    );
    harness
        .ctx
        .memory_mut(|memory| memory.request_focus(Id::new("pwds.music.name")));
    harness.draw(&snapshot, vec![Event::Text("Road trip".into())], true);
    let saved = harness.activate(&snapshot, "pwds.music.save", true);
    assert!(matches!(
        saved.commands.as_slice(),
        [AppCommand::Music(MusicCommand::SavePlaylist { name })] if name == "Road trip"
    ));
    assert_eq!(snapshot.playlists.len(), 1);
    let playlist = snapshot.playlists[0].id;
    harness.click(&snapshot, Id::new(("pwds.music.playlist", playlist)), true);
    let loaded = harness.activate(&snapshot, "pwds.music.load", true);
    assert!(matches!(
        loaded.commands.as_slice(),
        [AppCommand::Music(MusicCommand::LoadPlaylist(id))] if *id == playlist
    ));
    assert_eq!(snapshot.queue[0].title, "Zebra");
}

#[test]
fn overlong_paste_is_limited_before_a_save_intent_is_emitted() {
    let mut harness = Harness::default();
    let snapshot = snapshot();
    harness.draw(&snapshot, vec![], true);
    harness
        .ctx
        .memory_mut(|memory| memory.request_focus(Id::new("pwds.music.name")));
    harness.draw(&snapshot, vec![Event::Paste("\u{e9}".repeat(120))], true);
    let saved = harness.activate(&snapshot, "pwds.music.save", true);
    assert!(matches!(
        saved.commands.as_slice(),
        [AppCommand::Music(MusicCommand::SavePlaylist { name })]
            if name.chars().count() == 80 && name.chars().all(|character| character == '\u{e9}')
    ));
}

#[test]
fn queue_boundary_moves_are_disabled_and_remove_does_not_edit_the_snapshot() {
    let mut harness = Harness::default();
    let snapshot = snapshot();
    harness.draw(&snapshot, vec![], true);
    let first = snapshot.queue[0].id;
    harness.click(&snapshot, Id::new(("pwds.music.track", first)), true);
    assert!(
        harness
            .activate(&snapshot, "pwds.music.up", true)
            .commands
            .is_empty()
    );
    let removed = harness.activate(&snapshot, "pwds.music.remove", true);
    assert!(matches!(
        removed.commands.as_slice(),
        [AppCommand::Music(MusicCommand::RemoveTrack(id))] if *id == first
    ));
    assert_eq!(snapshot.queue.len(), 3);
    assert_eq!(snapshot.queue[0].id, first);
    harness.click(
        &snapshot,
        Id::new(("pwds.music.track", snapshot.queue[2].id)),
        true,
    );
    assert!(
        harness
            .activate(&snapshot, "pwds.music.down", true)
            .commands
            .is_empty()
    );
}

#[test]
fn tab_focus_reveals_offscreen_queue_and_playlist_rows_without_commands() {
    for playlists in [false, true] {
        let mut harness = Harness::default();
        let mut snapshot = snapshot();
        snapshot.queue = (1..=30)
            .map(|number| track(number, "Queue entry"))
            .collect();
        snapshot.playlists = (101..=130)
            .map(|number| Playlist {
                id: Uuid::from_u128(number),
                name: format!("Saved queue {number}"),
                tracks: Vec::new(),
            })
            .collect();
        harness.draw(&snapshot, vec![], true);
        let (kind, previous, target) = if playlists {
            ("pwds.music.playlist", 129, 130)
        } else {
            ("pwds.music.track", 29, 30)
        };
        let target = Id::new((kind, Uuid::from_u128(target)));
        let before = harness.ctx.read_response(target).expect("registered row");
        assert!(before.rect.top() > 560.0);
        harness.ctx.memory_mut(|memory| {
            memory.request_focus(Id::new((kind, Uuid::from_u128(previous))));
        });
        let tabbed = harness.draw(&snapshot, vec![key(Key::Tab, true)], true);
        assert!(tabbed.commands.is_empty());
        assert_eq!(harness.ctx.memory(|memory| memory.focused()), Some(target));
        assert!(
            harness
                .draw(&snapshot, vec![key(Key::Tab, false)], true)
                .commands
                .is_empty()
        );
        // Advance the real egui scrolling animation without a GPU or sleeping.
        for _ in 0..10 {
            assert!(harness.draw(&snapshot, vec![], true).commands.is_empty());
        }
        let revealed = harness.ctx.read_response(target).expect("focused row");
        assert!(revealed.interact_rect.contains(revealed.rect.center()));
        assert!(revealed.rect.top() < before.rect.top());
        assert_eq!(snapshot.queue[0].id, Uuid::from_u128(1));
        assert_eq!(snapshot.playlists[0].id, Uuid::from_u128(101));
    }
}

fn assert_queue_labels(frame: &Frame, expected: &[&str]) {
    let positions: Vec<_> = expected
        .iter()
        .map(|expected| {
            frame
                .labels
                .iter()
                .find(|(label, _)| label.as_str() == *expected)
                .expect("queue label is rendered")
                .1
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}
