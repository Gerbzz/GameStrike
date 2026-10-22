//! Native queue management. This view emits intents and never opens or plays media.
use super::{
    ControlState, activation::activated, control,
    paint::{LIGHT, MUTED, TEXT},
};
use crate::{
    application::AppCommand,
    music::{
        MusicCommand, MusicSnapshot,
        playback::{PlaybackCommand, PlaybackSnapshot, PlaybackState},
    },
};
use eframe::egui::{
    Align2, FontId, Id, PointerButton, Rect, ScrollArea, Sense, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use uuid::Uuid;

#[derive(Default)]
pub struct MusicPanel {
    selected_track: Option<Uuid>,
    selected_playlist: Option<Uuid>,
    playlist_name: String,
    queue_scroll: f32,
    playlist_scroll: f32,
    playback: PlaybackSnapshot,
    pending_selection: Option<Uuid>,
}

impl MusicPanel {
    /// Accept acknowledged controller state; rendering never starts playback.
    pub fn set_playback(&mut self, playback: &PlaybackSnapshot) {
        if !playback.available {
            self.pending_selection = None;
        } else if playback.revision != self.playback.revision || !self.playback.available {
            if self.pending_selection.is_some() && self.pending_selection == playback.selected {
                self.pending_selection = None;
                self.selected_track = playback.selected;
            } else if self.pending_selection.is_none()
                && (playback.selected != self.playback.selected || !self.playback.available)
            {
                self.selected_track = playback.selected;
            }
        }
        self.playback = playback.clone();
    }

    pub(super) fn playback_status(&self) -> &'static str {
        if !self.playback.available {
            return "PLAYBACK NOT CONNECTED";
        }
        match self.playback.state {
            PlaybackState::Unavailable => "PLAYBACK NOT CONNECTED",
            PlaybackState::Stopped => "PLAYBACK STOPPED",
            PlaybackState::Loading => "PLAYBACK LOADING",
            PlaybackState::Starting => "PLAYBACK STARTING",
            PlaybackState::Playing => "PLAYBACK PLAYING",
            PlaybackState::Pausing => "PLAYBACK PAUSING",
            PlaybackState::Paused => "PLAYBACK PAUSED",
            PlaybackState::Faulted => "PLAYBACK FAULTED",
        }
    }

    /// Draw the real panel without requiring a GPU-backed NativeView.
    /// `enabled` is the host's boot/input gate; store availability is checked here too.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        scale: f32,
        snapshot: &MusicSnapshot,
        enabled: bool,
    ) -> Vec<AppCommand> {
        let mut commands = Vec::new();
        if !rect.is_finite() || !rect.is_positive() {
            return commands;
        }
        let scale = if scale.is_finite() {
            scale.clamp(0.5, 1.5)
        } else {
            1.0
        };
        let gap = (12.0 * scale).max(8.0);
        let inner = rect.shrink(gap);
        let boot_enabled = enabled;
        let enabled = boot_enabled && snapshot.available;
        self.selected_track = self
            .selected_track
            .filter(|id| snapshot.queue.iter().any(|track| track.id == *id));
        self.pending_selection = self
            .pending_selection
            .filter(|id| snapshot.queue.iter().any(|track| track.id == *id));
        if self.selected_track.is_none() && self.playback.available {
            self.selected_track = self
                .playback
                .selected
                .filter(|id| snapshot.queue.iter().any(|track| track.id == *id));
        }
        self.selected_playlist = self
            .selected_playlist
            .filter(|id| snapshot.playlists.iter().any(|playlist| playlist.id == *id));
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner.intersect(ui.clip_rect()));
            let p = ui.painter().clone();
            p.text(
                inner.left_top(),
                Align2::LEFT_TOP,
                "LOCAL MUSIC QUEUE MANAGEMENT",
                FontId::monospace((19.0 * scale).max(12.0)),
                LIGHT,
            );
            p.text(
                inner.left_top() + vec2(0.0, (25.0 * scale).max(18.0)),
                Align2::LEFT_TOP,
                self.playback_status(),
                FontId::monospace((12.0 * scale).max(10.0)),
                MUTED,
            );
            let transport = Rect::from_min_size(
                inner.left_top() + vec2(0.0, (44.0 * scale).max(34.0)),
                vec2(inner.width(), (34.0 * scale).max(26.0)),
            );
            self.transport(ui, transport, snapshot, boot_enabled, scale, &mut commands);
            let current = self
                .playback
                .selected
                .and_then(|id| snapshot.queue.iter().position(|track| track.id == id));
            let current_title =
                current.map_or("NONE", |index| snapshot.queue[index].title.as_str());
            let next_title = current.map_or("NONE", |index| {
                snapshot
                    .queue
                    .get(index + 1)
                    .map_or("END OF QUEUE", |track| track.title.as_str())
            });
            let status = Rect::from_min_size(
                pos2(inner.left(), transport.bottom() + (8.0 * scale).max(6.0)),
                vec2(inner.width(), (20.0 * scale).max(16.0)),
            );
            p.with_clip_rect(status).text(
                status.left_top(),
                Align2::LEFT_TOP,
                format!("CURRENT: {current_title}  /  NEXT: {next_title}"),
                FontId::monospace((12.0 * scale).max(10.0)),
                MUTED,
            );
            let queue_problem = snapshot.problem.as_deref().or_else(|| {
                (!snapshot.available).then_some("Music queue storage is unavailable.")
            });
            let mut header_height = status.bottom() - inner.top() + (8.0 * scale).max(6.0);
            for problem in [queue_problem, self.playback.problem.as_deref()]
                .into_iter()
                .flatten()
            {
                let problem_rect = Rect::from_min_size(
                    inner.left_top() + vec2(0.0, header_height),
                    vec2(inner.width(), (24.0 * scale).max(18.0)),
                );
                p.with_clip_rect(problem_rect).text(
                    problem_rect.left_top(),
                    Align2::LEFT_TOP,
                    problem,
                    FontId::proportional((13.0 * scale).max(10.0)),
                    TEXT,
                );
                header_height += (26.0 * scale).max(20.0);
            }
            // Leave the existing global problem notice visible below both lists.
            let body = Rect::from_min_max(
                inner.min + vec2(0.0, header_height),
                inner.max - vec2(0.0, (30.0 * scale).max(24.0)),
            );
            if !body.is_positive() {
                return;
            }
            let split = body.left() + (body.width() - gap) * 0.57;
            let queue = Rect::from_min_max(body.min, pos2(split, body.bottom()));
            let playlists = Rect::from_min_max(pos2(split + gap, body.top()), body.max);
            self.queue(ui, queue, snapshot, enabled, scale, &mut commands);
            self.playlists(ui, playlists, snapshot, enabled, scale, &mut commands);
        });
        commands
    }

    pub(super) fn signature(&self) -> String {
        format!(
            "{:?}|{:?}|{}|{}|{}|{:?}|{:?}",
            self.selected_track,
            self.selected_playlist,
            self.playlist_name,
            self.queue_scroll.to_bits(),
            self.playlist_scroll.to_bits(),
            self.playback,
            self.pending_selection,
        )
    }

    fn transport(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        snapshot: &MusicSnapshot,
        boot_enabled: bool,
        scale: f32,
        commands: &mut Vec<AppCommand>,
    ) {
        let connected = boot_enabled && self.playback.available;
        let sources_enabled = connected && snapshot.available;
        let selected = self
            .selected_track
            .filter(|id| snapshot.queue.iter().any(|track| track.id == *id));
        let current = self
            .playback
            .selected
            .and_then(|id| snapshot.queue.iter().position(|track| track.id == id));
        let current_active = self.playback.selected == selected
            && matches!(
                self.playback.state,
                PlaybackState::Loading | PlaybackState::Starting | PlaybackState::Playing
            );
        let can_play = sources_enabled
            && selected.is_some()
            && self.playback.state != PlaybackState::Unavailable
            && !current_active;
        let can_pause = connected
            && matches!(
                self.playback.state,
                PlaybackState::Loading | PlaybackState::Starting | PlaybackState::Playing
            );
        let gap = (8.0 * scale).max(6.0);
        let width = (rect.width() - gap * 4.0) / 5.0;
        let actions = [
            (
                "pwds.music.previous",
                "PREVIOUS",
                sources_enabled && current.is_some_and(|i| i > 0),
            ),
            ("pwds.music.play", "PLAY", can_play),
            ("pwds.music.pause", "PAUSE", can_pause),
            ("pwds.music.stop", "STOP", connected),
            (
                "pwds.music.next",
                "NEXT",
                sources_enabled && current.is_some_and(|i| i + 1 < snapshot.queue.len()),
            ),
        ];
        for (index, (id, label, can_use)) in actions.into_iter().enumerate() {
            let action = Rect::from_min_size(
                rect.min + vec2(index as f32 * (width + gap), 0.0),
                vec2(width, rect.height()),
            );
            if button(ui, action, id, label, can_use, scale) {
                let command = match index {
                    0 => {
                        self.pending_selection = None;
                        PlaybackCommand::Previous
                    }
                    1 => {
                        let Some(id) = selected else { continue };
                        if self.pending_selection.is_some() || self.playback.selected != Some(id) {
                            self.pending_selection = Some(id);
                        }
                        PlaybackCommand::Play(id)
                    }
                    2 => PlaybackCommand::Pause,
                    3 => PlaybackCommand::Stop,
                    _ => {
                        self.pending_selection = None;
                        PlaybackCommand::Next
                    }
                };
                commands.push(AppCommand::Playback(command));
            }
        }
    }

    fn queue(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        snapshot: &MusicSnapshot,
        enabled: bool,
        scale: f32,
        commands: &mut Vec<AppCommand>,
    ) {
        let gap = (8.0 * scale).max(6.0);
        let height = (34.0 * scale).max(26.0);
        let add = Rect::from_min_size(
            pos2(rect.center().x, rect.top()),
            vec2(rect.width() * 0.5, height),
        );
        ui.painter().text(
            rect.left_top() + vec2(0.0, height * 0.5),
            Align2::LEFT_CENTER,
            format!("QUEUE ({})", snapshot.queue.len()),
            FontId::monospace((16.0 * scale).max(11.0)),
            LIGHT,
        );
        if button(ui, add, "pwds.music.add", "ADD LOCAL FILES", enabled, scale) {
            commands.push(AppCommand::ChooseMusicFiles);
        }
        let list = Rect::from_min_max(
            rect.min + vec2(0.0, height + gap),
            rect.max - vec2(0.0, height + gap),
        );
        if list.is_positive() {
            let output = ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                ui.set_clip_rect(list.intersect(ui.clip_rect()));
                ScrollArea::vertical()
                    .id_salt("pwds.music.queue.scroll")
                    .max_height(list.height())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if snapshot.queue.is_empty() {
                            ui.label("Queue is empty. Add local files to begin.");
                        }
                        for (index, track) in snapshot.queue.iter().enumerate() {
                            let (row, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), height),
                                Sense::hover(),
                            );
                            let label = format!("{:02}  {}", index + 1, track.title);
                            if selectable_row(
                                ui,
                                row,
                                Id::new(("pwds.music.track", track.id)),
                                &label,
                                enabled,
                                self.selected_track == Some(track.id),
                                scale,
                            ) {
                                self.selected_track = Some(track.id);
                                if self.playback.available {
                                    if self.pending_selection.is_some()
                                        || self.playback.selected != Some(track.id)
                                    {
                                        self.pending_selection = Some(track.id);
                                    }
                                    commands.push(AppCommand::Playback(PlaybackCommand::Select(
                                        track.id,
                                    )));
                                }
                            }
                        }
                    })
            });
            self.queue_scroll = output.inner.state.offset.y;
        }
        let selected = self
            .selected_track
            .and_then(|id| snapshot.queue.iter().position(|track| track.id == id));
        let width = (rect.width() - gap * 2.0) / 3.0;
        for (index, (id, label, can_use)) in [
            (
                "pwds.music.up",
                "UP",
                enabled && selected.is_some_and(|index| index > 0),
            ),
            (
                "pwds.music.down",
                "DOWN",
                enabled && selected.is_some_and(|index| index + 1 < snapshot.queue.len()),
            ),
            (
                "pwds.music.remove",
                "REMOVE",
                enabled && selected.is_some(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let action = Rect::from_min_size(
                pos2(
                    rect.left() + index as f32 * (width + gap),
                    rect.bottom() - height,
                ),
                vec2(width, height),
            );
            if button(ui, action, id, label, can_use, scale)
                && let Some(selected) = self.selected_track
            {
                let command = match index {
                    0 => MusicCommand::MoveTrack {
                        id: selected,
                        up: true,
                    },
                    1 => MusicCommand::MoveTrack {
                        id: selected,
                        up: false,
                    },
                    _ => MusicCommand::RemoveTrack(selected),
                };
                commands.push(AppCommand::Music(command));
            }
        }
    }

    fn playlists(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        snapshot: &MusicSnapshot,
        enabled: bool,
        scale: f32,
        commands: &mut Vec<AppCommand>,
    ) {
        let gap = (8.0 * scale).max(6.0);
        let height = (34.0 * scale).max(26.0);
        let load = Rect::from_min_size(
            rect.left_bottom() - vec2(0.0, height),
            vec2(rect.width(), height),
        );
        let save = load.translate(vec2(0.0, -height - gap));
        let name = save.translate(vec2(0.0, -height - gap));
        let caption_height = (20.0 * scale).max(16.0);
        let list = Rect::from_min_max(
            rect.min + vec2(0.0, height + gap),
            pos2(rect.right(), name.top() - caption_height - gap),
        );
        ui.painter().text(
            rect.left_top() + vec2(0.0, height * 0.5),
            Align2::LEFT_CENTER,
            format!("SAVED PLAYLISTS ({})", snapshot.playlists.len()),
            FontId::monospace((16.0 * scale).max(11.0)),
            LIGHT,
        );
        if list.is_positive() {
            let output = ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                ui.set_clip_rect(list.intersect(ui.clip_rect()));
                ScrollArea::vertical()
                    .id_salt("pwds.music.playlists.scroll")
                    .max_height(list.height())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if snapshot.playlists.is_empty() {
                            ui.label("No saved playlists.");
                        }
                        for playlist in &snapshot.playlists {
                            let (row, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), height),
                                Sense::hover(),
                            );
                            let label =
                                format!("{}  ({} TRACKS)", playlist.name, playlist.tracks.len());
                            if selectable_row(
                                ui,
                                row,
                                Id::new(("pwds.music.playlist", playlist.id)),
                                &label,
                                enabled,
                                self.selected_playlist == Some(playlist.id),
                                scale,
                            ) {
                                self.selected_playlist = Some(playlist.id);
                            }
                        }
                    })
            });
            self.playlist_scroll = output.inner.state.offset.y;
        }
        ui.painter().text(
            name.left_top() - vec2(0.0, caption_height),
            Align2::LEFT_TOP,
            "PLAYLIST NAME",
            FontId::monospace((12.0 * scale).max(10.0)),
            MUTED,
        );
        ui.add_enabled_ui(enabled, |ui| {
            ui.put(
                name,
                TextEdit::singleline(&mut self.playlist_name)
                    .id(Id::new("pwds.music.name"))
                    .char_limit(80)
                    .hint_text("Name this queue"),
            );
        });
        let name = self.playlist_name.trim();
        let can_save = enabled
            && !snapshot.queue.is_empty()
            && !name.is_empty()
            && !name.chars().any(char::is_control);
        if button(ui, save, "pwds.music.save", "SAVE QUEUE", can_save, scale) {
            commands.push(AppCommand::Music(MusicCommand::SavePlaylist {
                name: name.to_owned(),
            }));
        }
        if button(
            ui,
            load,
            "pwds.music.load",
            "LOAD SELECTED",
            enabled && self.selected_playlist.is_some(),
            scale,
        )
            && let Some(id) = self.selected_playlist
        {
            commands.push(AppCommand::Music(MusicCommand::LoadPlaylist(id)));
        }
    }
}

fn button(ui: &mut Ui, rect: Rect, id: &str, label: &str, enabled: bool, scale: f32) -> bool {
    selectable_row(ui, rect, Id::new(id), label, enabled, false, scale)
}

fn selectable_row(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label: &str,
    enabled: bool,
    selected: bool,
    scale: f32,
) -> bool {
    let p = ui.painter().clone();
    let response = control(
        ui,
        &p,
        rect,
        id,
        label,
        ControlState { enabled, selected },
        5.0 * scale,
    );
    if enabled && response.gained_focus() {
        response.scroll_to_me(None);
    }
    // A clipped row keeps its pointer surface inside the list; keyboard activation
    // still follows the same production control path without a pointing device.
    if response.clicked_by(PointerButton::Primary)
        && response
            .interact_pointer_pos()
            .is_some_and(|point| !ui.clip_rect().contains(point))
    {
        return false;
    }
    let active = activated(ui, &response, rect, 5.0 * scale, enabled);
    if active {
        response.request_focus();
    }
    active
}
