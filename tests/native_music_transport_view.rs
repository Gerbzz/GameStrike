#![cfg(feature = "native-ui")]

use eframe::egui::{
    Context, Event, Id, Key, Modifiers, PointerButton, RawInput, Rect, Ui, UiBuilder, pos2, vec2,
};
use gamestrike::{
    application::AppCommand,
    music::{
        MusicCommand, MusicSnapshot, Track,
        playback::{PlaybackCommand, PlaybackSnapshot, PlaybackState},
    },
    native_ui::music::MusicPanel,
};
use uuid::Uuid;

fn queue() -> MusicSnapshot {
    MusicSnapshot {
        queue: ["Zebra", "Moon", "Amber"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| Track {
                id: Uuid::from_u128(index as u128 + 1),
                title: title.into(),
                path: format!("owned-transport-fixture-{index}.wav").into(),
            })
            .collect(),
        available: true,
        revision: 1,
        ..Default::default()
    }
}

fn attached(state: PlaybackState, selected: u128, revision: u64) -> PlaybackSnapshot {
    PlaybackSnapshot {
        available: true,
        state,
        selected: Some(Uuid::from_u128(selected)),
        revision,
        problem: None,
    }
}

struct Frame {
    commands: Vec<AppCommand>,
    labels: Vec<String>,
}

#[derive(Default)]
struct Harness {
    ctx: Context,
    panel: MusicPanel,
    time: f64,
}

impl Harness {
    fn draw(&mut self, queue: &MusicSnapshot, events: Vec<Event>, enabled: bool) -> Frame {
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
            Id::new("native.music.transport.test.root"),
            UiBuilder::new().max_rect(rect),
        );
        let commands = self.panel.show(&mut ui, rect, 1.0, queue, enabled);
        drop(ui);
        let output = self.ctx.end_pass();
        let mut labels = Vec::new();
        for clipped in &output.shapes {
            collect_labels(&clipped.shape, &mut labels);
        }
        output.drop_without_applying_deltas();
        Frame { commands, labels }
    }

    fn activate(&mut self, queue: &MusicSnapshot, id: Id, enabled: bool) -> Frame {
        self.ctx.memory_mut(|memory| memory.request_focus(id));
        let pressed = self.draw(queue, vec![key(true)], enabled);
        let released = self.draw(queue, vec![key(false)], enabled);
        assert!(released.commands.is_empty());
        pressed
    }

    fn click(&mut self, queue: &MusicSnapshot, id: Id, enabled: bool) -> Frame {
        let point = self
            .ctx
            .read_response(id)
            .expect("Music control")
            .rect
            .center();
        let pressed = self.draw(
            queue,
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
            queue,
            vec![Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            enabled,
        )
    }
}

fn collect_labels(shape: &eframe::egui::Shape, labels: &mut Vec<String>) {
    match shape {
        eframe::egui::Shape::Text(text) => labels.push(text.galley.text().to_owned()),
        eframe::egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_labels(shape, labels);
            }
        }
        _ => {}
    }
}

fn key(pressed: bool) -> Event {
    Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn assert_playback(frame: &Frame, expected: PlaybackCommand) {
    assert!(matches!(
        frame.commands.as_slice(),
        [AppCommand::Playback(command)] if *command == expected
    ));
}

#[test]
fn row_selection_never_autoplays_and_play_keeps_the_explicit_row_after_select_failure() {
    let mut harness = Harness::default();
    let queue = queue();
    let mut playback = attached(PlaybackState::Stopped, 1, 1);
    harness.panel.set_playback(&playback);
    harness.draw(&queue, vec![], true);
    let chosen = Uuid::from_u128(2);
    let selected = harness.click(&queue, Id::new(("pwds.music.track", chosen)), true);
    assert_playback(&selected, PlaybackCommand::Select(chosen));

    playback.problem = Some("Owned fixture selection request failed".into());
    playback.revision += 1;
    harness.panel.set_playback(&playback);
    let played = harness.activate(&queue, Id::new("pwds.music.play"), true);
    assert_playback(&played, PlaybackCommand::Play(chosen));
    assert!(
        played
            .labels
            .iter()
            .any(|label| label == "PLAYBACK STOPPED")
    );
    assert!(harness.draw(&queue, vec![], true).commands.is_empty());
    assert_eq!(queue.queue[0].id, Uuid::from_u128(1));
}

#[test]
fn keyboard_row_selection_emits_one_select_intent() {
    let mut harness = Harness::default();
    let queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 1, 1));
    harness.draw(&queue, vec![], true);
    let selected = Uuid::from_u128(3);
    let frame = harness.activate(&queue, Id::new(("pwds.music.track", selected)), true);
    assert_playback(&frame, PlaybackCommand::Select(selected));
    assert!(harness.draw(&queue, vec![], true).commands.is_empty());
}

#[test]
fn displayed_playing_requires_an_authoritative_started_snapshot() {
    let mut harness = Harness::default();
    let queue = queue();
    for (revision, state, label) in [
        (1, PlaybackState::Loading, "PLAYBACK LOADING"),
        (2, PlaybackState::Starting, "PLAYBACK STARTING"),
    ] {
        harness.panel.set_playback(&attached(state, 1, revision));
        let frame = harness.draw(&queue, vec![], true);
        assert!(frame.labels.iter().any(|text| text == label));
        assert!(!frame.labels.iter().any(|text| text == "PLAYBACK PLAYING"));
        assert!(
            harness
                .activate(&queue, Id::new("pwds.music.play"), true)
                .commands
                .is_empty()
        );
    }
    harness
        .panel
        .set_playback(&attached(PlaybackState::Playing, 1, 3));
    let playing = harness.draw(&queue, vec![], true);
    assert!(playing.labels.iter().any(|text| text == "PLAYBACK PLAYING"));
    let pause = harness.click(&queue, Id::new("pwds.music.pause"), true);
    assert_playback(&pause, PlaybackCommand::Pause);
    assert!(pause.labels.iter().any(|text| text == "PLAYBACK PLAYING"));
    harness
        .panel
        .set_playback(&attached(PlaybackState::Pausing, 1, 4));
    let pausing = harness.draw(&queue, vec![], true);
    assert!(pausing.labels.iter().any(|text| text == "PLAYBACK PAUSING"));
    harness
        .panel
        .set_playback(&attached(PlaybackState::Paused, 1, 5));
    let paused = harness.draw(&queue, vec![], true);
    assert!(paused.labels.iter().any(|text| text == "PLAYBACK PAUSED"));
}

#[test]
fn transport_pointer_and_keyboard_actions_emit_only_the_exact_intent() {
    let mut harness = Harness::default();
    let queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Playing, 2, 1));
    harness.draw(&queue, vec![], true);
    for (id, command, pointer) in [
        ("pwds.music.pause", PlaybackCommand::Pause, false),
        ("pwds.music.stop", PlaybackCommand::Stop, true),
        ("pwds.music.next", PlaybackCommand::Next, false),
        ("pwds.music.previous", PlaybackCommand::Previous, true),
    ] {
        let frame = if pointer {
            harness.click(&queue, Id::new(id), true)
        } else {
            harness.activate(&queue, Id::new(id), true)
        };
        assert_playback(&frame, command);
    }
    assert_eq!(queue.queue[1].title, "Moon");
}

#[test]
fn unavailable_transport_keeps_queue_management_and_local_selection_available() {
    let mut harness = Harness::default();
    let queue = queue();
    let frame = harness.draw(&queue, vec![], true);
    assert!(
        frame
            .labels
            .iter()
            .any(|label| label == "PLAYBACK NOT CONNECTED")
    );
    let chosen = Uuid::from_u128(2);
    assert!(
        harness
            .click(&queue, Id::new(("pwds.music.track", chosen)), true)
            .commands
            .is_empty()
    );
    harness.panel.set_playback(&PlaybackSnapshot {
        revision: 1,
        ..Default::default()
    });
    let remove = harness.activate(&queue, Id::new("pwds.music.remove"), true);
    assert!(matches!(
        remove.commands.as_slice(),
        [AppCommand::Music(MusicCommand::RemoveTrack(id))] if *id == chosen
    ));
    let add = harness.activate(&queue, Id::new("pwds.music.add"), true);
    assert!(matches!(
        add.commands.as_slice(),
        [AppCommand::ChooseMusicFiles]
    ));
    for id in transport_ids() {
        assert!(
            harness
                .activate(&queue, Id::new(id), true)
                .commands
                .is_empty()
        );
        assert!(harness.click(&queue, Id::new(id), true).commands.is_empty());
    }
}

#[test]
fn boot_disables_transport_and_row_intents() {
    let mut harness = Harness::default();
    let queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Playing, 2, 1));
    harness.draw(&queue, vec![], false);
    for id in transport_ids() {
        assert!(
            harness
                .activate(&queue, Id::new(id), false)
                .commands
                .is_empty()
        );
        assert!(
            harness
                .click(&queue, Id::new(id), false)
                .commands
                .is_empty()
        );
    }
    assert!(
        harness
            .activate(
                &queue,
                Id::new(("pwds.music.track", Uuid::from_u128(3))),
                false
            )
            .commands
            .is_empty()
    );
}

#[test]
fn storage_failure_disables_source_actions_but_retains_connected_pause_and_stop() {
    let mut harness = Harness::default();
    let mut queue = queue();
    queue.available = false;
    harness
        .panel
        .set_playback(&attached(PlaybackState::Playing, 2, 1));
    harness.draw(&queue, vec![], true);
    for id in ["pwds.music.play", "pwds.music.next", "pwds.music.previous"] {
        assert!(
            harness
                .activate(&queue, Id::new(id), true)
                .commands
                .is_empty()
        );
    }
    let pause = harness.activate(&queue, Id::new("pwds.music.pause"), true);
    assert_playback(&pause, PlaybackCommand::Pause);
    let stop = harness.click(&queue, Id::new("pwds.music.stop"), true);
    assert_playback(&stop, PlaybackCommand::Stop);
}

#[test]
fn delayed_selection_acknowledgements_preserve_the_newest_local_row_choice() {
    let mut harness = Harness::default();
    let queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 1, 1));
    harness.draw(&queue, vec![], true);
    for selected in [2, 3] {
        let id = Uuid::from_u128(selected);
        let frame = harness.click(&queue, Id::new(("pwds.music.track", id)), true);
        assert_playback(&frame, PlaybackCommand::Select(id));
    }
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 2, 2));
    let before_latest_ack = harness.activate(&queue, Id::new("pwds.music.play"), true);
    assert_playback(
        &before_latest_ack,
        PlaybackCommand::Play(Uuid::from_u128(3)),
    );
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 3, 3));
    let after_latest_ack = harness.activate(&queue, Id::new("pwds.music.play"), true);
    assert_playback(&after_latest_ack, PlaybackCommand::Play(Uuid::from_u128(3)));
}

#[test]
fn removed_pending_row_falls_back_to_the_acknowledged_retained_selection() {
    let mut harness = Harness::default();
    let mut queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 1, 1));
    harness.draw(&queue, vec![], true);
    harness.click(
        &queue,
        Id::new(("pwds.music.track", Uuid::from_u128(3))),
        true,
    );
    queue.queue.pop();
    queue.revision += 1;
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 2, 2));
    harness.draw(&queue, vec![], true);
    let play = harness.activate(&queue, Id::new("pwds.music.play"), true);
    assert_playback(&play, PlaybackCommand::Play(Uuid::from_u128(2)));
}

#[test]
fn reselecting_the_acknowledged_row_does_not_hide_later_controller_advancement() {
    let mut harness = Harness::default();
    let queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Playing, 1, 1));
    harness.draw(&queue, vec![], true);
    let selected = harness.click(
        &queue,
        Id::new(("pwds.music.track", Uuid::from_u128(1))),
        true,
    );
    assert_playback(&selected, PlaybackCommand::Select(Uuid::from_u128(1)));
    // Selecting the already selected row is a controller no-op with no new revision.
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 2, 2));
    let play = harness.activate(&queue, Id::new("pwds.music.play"), true);
    assert_playback(&play, PlaybackCommand::Play(Uuid::from_u128(2)));
}

#[test]
fn next_and_previous_wait_for_authoritative_selection_and_respect_queue_ends() {
    let mut harness = Harness::default();
    let queue = queue();
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 1, 1));
    let initial = harness.draw(&queue, vec![], true);
    assert!(
        initial
            .labels
            .iter()
            .any(|label| label == "CURRENT: Zebra  /  NEXT: Moon")
    );
    assert!(
        harness
            .activate(&queue, Id::new("pwds.music.previous"), true)
            .commands
            .is_empty()
    );
    let next = harness.activate(&queue, Id::new("pwds.music.next"), true);
    assert_playback(&next, PlaybackCommand::Next);
    let before_commit = harness.draw(&queue, vec![], true);
    assert!(
        before_commit
            .labels
            .iter()
            .any(|label| label == "CURRENT: Zebra  /  NEXT: Moon")
    );
    assert!(before_commit.commands.is_empty());
    harness
        .panel
        .set_playback(&attached(PlaybackState::Stopped, 3, 2));
    let committed = harness.draw(&queue, vec![], true);
    assert!(
        committed
            .labels
            .iter()
            .any(|label| label == "CURRENT: Amber  /  NEXT: END OF QUEUE")
    );
    assert!(
        harness
            .activate(&queue, Id::new("pwds.music.next"), true)
            .commands
            .is_empty()
    );
    let play = harness.activate(&queue, Id::new("pwds.music.play"), true);
    assert_playback(&play, PlaybackCommand::Play(Uuid::from_u128(3)));
}

#[test]
fn playback_failure_text_is_visible_without_hiding_queue_failure_text() {
    let mut harness = Harness::default();
    let mut queue = queue();
    queue.problem = Some("Owned fixture queue failure".into());
    let mut playback = attached(PlaybackState::Faulted, 2, 2);
    playback.problem = Some("Owned fixture transport failure".into());
    harness.panel.set_playback(&playback);
    let frame = harness.draw(&queue, vec![], true);
    for expected in [
        "PLAYBACK FAULTED",
        "Owned fixture queue failure",
        "Owned fixture transport failure",
    ] {
        assert!(frame.labels.iter().any(|label| label == expected));
    }
    assert!(frame.commands.is_empty());
}

fn transport_ids() -> [&'static str; 5] {
    [
        "pwds.music.play",
        "pwds.music.pause",
        "pwds.music.stop",
        "pwds.music.next",
        "pwds.music.previous",
    ]
}
