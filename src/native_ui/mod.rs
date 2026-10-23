//! Native PWDS Collection view. This module returns application intents only.
//! It has no process, file-picker, browser, network, or audio-device authority.
mod activation;
pub mod boot;
mod cache_budget;
mod covers;
mod fonts;
pub mod geometry;
mod gpu;
mod mesh_geometry;
pub mod music;
mod navigation;
mod paint;
pub mod sidebar;

use crate::application::{AppCommand, AppSnapshot, GameCard, GameId, GameOrigin};
use activation::activated;
use boot::BootPose;
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Response, Sense,
    Shape, Stroke, TextureHandle, WidgetInfo, WidgetType, pos2, vec2,
};
use geometry::{Geometry, chamfer};
use paint::{AMBER, LIGHT, MUTED, TEXT};
use std::sync::atomic::Ordering;

const SECTIONS: [&str; 4] = ["GAMES", "SOUNDBOARD", "VOICE", "MUSIC"];

/// Install the fixed theme and bundled fonts before the first egui pass.
/// Standalone native panels use the same setup as the full GPU-backed view.
pub fn configure_context(ctx: &egui::Context) {
    configure_visuals(ctx);
    fonts::install(ctx);
}

fn configure_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.selection.bg_fill = AMBER;
    visuals.selection.stroke = Stroke::new(1.0, LIGHT);
    visuals.override_text_color = Some(TEXT);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals(visuals);
}

struct PaintContext<'a> {
    painter: &'a Painter,
    geometry: &'a Geometry,
}

struct ControlState {
    enabled: bool,
    selected: bool,
}

pub struct NativeView {
    gpu: gpu::NativeGpu,
    covers: covers::Covers,
    logo: TextureHandle,
    music: music::MusicPanel,
    section: usize,
    game_sidebar: sidebar::GamesSidebar,
    selected: Option<GameId>,
    scroll: f32,
    epoch: u64,
    frame_signature: String,
    textures_prepared: bool,
}

impl NativeView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let state = cc.wgpu_render_state.as_ref().ok_or_else(|| {
            "PWDS requires the native wgpu renderer; no compatible render state was supplied"
                .to_owned()
        })?;
        let gpu = gpu::NativeGpu::new(state)?;
        let logo = covers::logo(&cc.egui_ctx)?;
        configure_context(&cc.egui_ctx);
        Ok(Self {
            gpu,
            covers: covers::Covers::default(),
            logo,
            music: music::MusicPanel::default(),
            section: 0,
            game_sidebar: sidebar::GamesSidebar::default(),
            selected: None,
            scroll: 0.0,
            epoch: 0,
            frame_signature: String::new(),
            textures_prepared: false,
        })
    }

    /// Select an existing route for a synthetic framebuffer case; no intent is emitted.
    pub fn select_capture_section(&mut self, section: crate::native_capture::CaptureSection) {
        self.section = section.index();
    }

    pub fn section_name(&self) -> &'static str {
        ["games", "soundboard", "voice", "music"][self.section]
    }

    pub fn renderer_label(&self) -> String {
        self.gpu.label.clone()
    }

    pub fn set_playback(&mut self, playback: &crate::music::playback::PlaybackSnapshot) {
        self.music.set_playback(playback);
    }

    /// True only after this exact content pose has prepared texture data and a
    /// native callback has recorded its foreground draw in a real render pass.
    /// The host must separately wait for capture/readback or presentation.
    pub fn content_ready(&self) -> bool {
        self.epoch > 0
            && self.textures_prepared
            && self.gpu.receipt.prepared.load(Ordering::Acquire) == self.epoch
            && self.gpu.receipt.painted.load(Ordering::Acquire) == self.epoch
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        snapshot: &AppSnapshot,
        pose: BootPose,
    ) -> Vec<AppCommand> {
        // Public pose fields cannot inject NaN into geometry or uniforms.
        let pose = BootPose::at(pose.seconds, false, false);
        let g = Geometry::new(ctx.content_rect(), ctx.pixels_per_point());
        let mut commands = Vec::new();
        self.covers.retain(&snapshot.games);
        self.textures_prepared = false;
        egui::Area::new(Id::new("pwds.native.collection"))
            .order(egui::Order::Background)
            .fixed_pos(g.viewport.min)
            .movable(false)
            .sense(Sense::hover())
            .fade_in(false)
            .constrain(false)
            .default_size(g.viewport.size())
            .show(ctx, |ui| {
                ui.set_min_size(g.viewport.size());
                ui.set_clip_rect(g.viewport);
                let p = ui.painter().clone();
                p.add(self.gpu.callback(g.viewport, pose, false, 0));
                let chrome = paint::faded(&p, pose.frame);
                paint::frame(&chrome, &g, pose);
                if pose.content > 0.0 {
                    let content = paint::faded(&p, pose.content);
                    commands.extend(self.sidebar(ui, &content, &g, snapshot, pose));
                    self.heading(&content, &g, snapshot);
                }
                let visible = self.visible(snapshot);
                if !visible
                    .iter()
                    .any(|&i| Some(&snapshot.games[i].id) == self.selected.as_ref())
                {
                    self.selected = visible.first().map(|&i| snapshot.games[i].id.clone());
                }
                self.scroll = self.scroll.clamp(0.0, g.max_scroll(visible.len()));
                if self.section == 0 && pose.settled() {
                    self.keyboard(ctx, &g, snapshot, &visible, &mut commands);
                    if ctx.input(|i| {
                        i.pointer
                            .hover_pos()
                            .is_some_and(|point| g.library.contains(point))
                    }) {
                        self.scroll = (self.scroll - ctx.input(|i| i.smooth_scroll_delta.y))
                            .clamp(0.0, g.max_scroll(visible.len()));
                    }
                }
                if self.section == 0 {
                    // Prepare actual CPU image data even during the black ignition
                    // frame; channel enqueue alone never sets content_ready.
                    for (position, &i) in visible.iter().enumerate() {
                        let rect = g.card_rect(position, self.scroll);
                        if rect.intersects(g.library) {
                            self.covers
                                .prepare(ctx, &snapshot.games[i], snapshot.revision);
                        }
                    }
                    self.covers.trim();
                    self.textures_prepared = visible.iter().enumerate().all(|(position, &i)| {
                        !g.card_rect(position, self.scroll).intersects(g.library)
                            || self.covers.textures.contains_key(&snapshot.games[i].id)
                    });
                    if pose.content > 0.0 {
                        let content = paint::faded(&p, pose.content);
                        self.games(
                            ui,
                            PaintContext {
                                painter: &content,
                                geometry: &g,
                            },
                            snapshot,
                            &visible,
                            pose.settled(),
                            &mut commands,
                        );
                    }
                } else {
                    self.textures_prepared = true;
                    if pose.content > 0.0 {
                        if self.section == 3 {
                            ui.scope_builder(egui::UiBuilder::new().max_rect(g.library), |ui| {
                                ui.set_opacity(pose.content);
                                commands.extend(self.music.show(
                                    ui,
                                    g.library,
                                    g.scale,
                                    &snapshot.music,
                                    pose.settled(),
                                ));
                            });
                        } else {
                            self.unavailable(&paint::faded(&p, pose.content), &g);
                        }
                    }
                }
                if pose.content > 0.0 {
                    self.bottom(
                        ui,
                        &paint::faded(&p, pose.content),
                        &g,
                        snapshot,
                        pose.settled(),
                        &mut commands,
                    );
                }
                paint::emblem(&p, &g, pose, self.logo.id());
                if !pose.settled() && pose.seconds >= 3.3 && pose.seconds < 4.65 {
                    let alpha = boot::smooth(3.3, 3.65, pose.seconds)
                        * (1.0 - boot::smooth(4.2, 4.65, pose.seconds));
                    let at = pos2(
                        g.viewport.center().x,
                        g.viewport.top() + g.viewport.height() * 0.65,
                    );
                    paint::faded(&p, alpha).text(
                        at,
                        Align2::CENTER_CENTER,
                        "GAMESTRIKE",
                        fonts::display(42.0 * g.scale.max(0.5)),
                        Color32::from_rgb(255, 196, 122),
                    );
                }
                let signature = format!(
                    "{}|{}|{:?}|{}|{:?}|{:?}|{}|{}|{:?}|{}|{}|{:?}|{}|{:?}",
                    snapshot.revision,
                    snapshot.games.len(),
                    g.viewport,
                    self.section,
                    self.game_sidebar.filter,
                    self.selected,
                    self.scroll,
                    g.pixels_per_point,
                    pose,
                    snapshot.music.revision,
                    snapshot.music.available,
                    snapshot.music.problem,
                    self.music.signature(),
                    snapshot.problems,
                );
                if self.frame_signature != signature {
                    self.epoch = self.epoch.checked_add(1).unwrap_or(1);
                    self.frame_signature = signature;
                }
                // Last callback: after covers, native text, focus rings and emblem.
                p.add(self.gpu.callback(g.viewport, pose, true, self.epoch));
            });
        // The host owns live animation/capture cadence. Frozen poses become idle
        // once the actual render callback has acknowledged their content.
        if !self.content_ready() {
            ctx.request_repaint();
        }
        commands
    }

    fn visible(&self, snapshot: &AppSnapshot) -> Vec<usize> {
        self.game_sidebar.visible(snapshot)
    }

    fn heading(&self, p: &Painter, g: &Geometry, snapshot: &AppSnapshot) {
        let title = if self.section == 0 {
            "GAME COLLECTION"
        } else {
            SECTIONS[self.section]
        };
        let title_rect = g.header.shrink2(vec2(16.0, 0.0) * g.scale);
        paint::display_title(&p.with_clip_rect(g.header), title_rect, title, g.scale);
        let info = if self.section == 3 {
            format!(
                "{} QUEUED  /  {} PLAYLISTS",
                snapshot.music.queue.len(),
                snapshot.music.playlists.len()
            )
        } else if self.section != 0 {
            "NOT CONNECTED IN THIS BUILD".to_owned()
        } else if snapshot.refreshing {
            "REFRESHING COLLECTION".to_owned()
        } else if snapshot.synthetic {
            format!("SYNTHETIC LIBRARY  /  {} GAMES", snapshot.games.len())
        } else {
            format!(
                "{} OF {} GAMES  /  {}",
                self.visible(snapshot).len(),
                snapshot.games.len(),
                self.game_sidebar.filter.label()
            )
        };
        let available = Rect::from_min_max(
            pos2(g.header.right() + 60.0 * g.scale, g.header.top()),
            pos2(g.frame.right() - 65.0 * g.scale, g.header.bottom()),
        );
        if available.is_positive() {
            p.with_clip_rect(available).text(
                available.center(),
                Align2::CENTER_CENTER,
                info,
                FontId::monospace((13.0 * g.scale).max(9.0)),
                MUTED,
            );
        }
    }

    fn sidebar(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        g: &Geometry,
        snapshot: &AppSnapshot,
        pose: BootPose,
    ) -> Vec<AppCommand> {
        let enabled = pose.settled();
        let focused_at_start = ui.memory(|memory| memory.focused());
        for (i, label) in SECTIONS.iter().enumerate() {
            let rect = g.navigation[i];
            let response = control(
                ui,
                p,
                rect,
                nav_id(i),
                label,
                ControlState {
                    enabled,
                    selected: self.section == i,
                },
                8.0 * g.scale,
            );
            glyph(
                p,
                pos2(rect.left() + 19.0 * g.scale, rect.center().y),
                11.0 * g.scale,
                i,
                if self.section == i {
                    Color32::from_rgb(22, 6, 0)
                } else {
                    TEXT
                },
            );
            if activated(ui, &response, rect, 8.0 * g.scale, enabled) {
                self.section = i;
                response.request_focus();
            }
            if enabled && focused_at_start == Some(response.id) {
                let action = ui.input_mut(|input| navigation::take_nav_action(input, i));
                if let Some(action) = action {
                    // egui also schedules an automatic directional move at end_pass.
                    // Clear it after taking ownership of this key, then request one target.
                    ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                    match action {
                        navigation::NavAction::Select(next) => {
                            self.section = next;
                            ui.memory_mut(|memory| memory.request_focus(nav_id(next)));
                        }
                        navigation::NavAction::EnterGames => {
                            if let Some(id) = &self.selected {
                                ui.memory_mut(|memory| memory.request_focus(card_id(id)));
                            }
                        }
                    }
                }
            }
        }
        let previous_filter = self.game_sidebar.filter;
        let output = ui.scope_builder(egui::UiBuilder::new().max_rect(g.context), |ui| {
            ui.set_clip_rect(g.context.intersect(ui.clip_rect()));
            ui.set_opacity(pose.content);
            match self.section {
                0 => self
                    .game_sidebar
                    .show(ui, g.context, g.scale, snapshot, enabled),
                3 => self.music.show_sidebar(
                    ui,
                    g.context,
                    g.scale,
                    &snapshot.music,
                    enabled && !snapshot.synthetic,
                ),
                section => {
                    sidebar::unavailable(ui, g.context, g.scale, section);
                    Vec::new()
                }
            }
        });
        if self.game_sidebar.filter != previous_filter {
            self.scroll = 0.0;
        }
        output.inner
    }

    fn keyboard(
        &mut self,
        ctx: &egui::Context,
        g: &Geometry,
        snapshot: &AppSnapshot,
        visible: &[usize],
        commands: &mut Vec<AppCommand>,
    ) {
        let focused = ctx.memory(|m| m.focused());
        let grid_focused = focused.is_none()
            || visible
                .iter()
                .any(|&i| Some(card_id(&snapshot.games[i].id)) == focused);
        if ctx.input(|i| i.key_pressed(Key::R) && !i.modifiers.command && !i.modifiers.alt)
            && !snapshot.refreshing
        {
            commands.push(AppCommand::Refresh);
        }
        if !grid_focused || visible.is_empty() {
            return;
        }
        let index = visible
            .iter()
            .position(|&i| Some(&snapshot.games[i].id) == self.selected.as_ref())
            .unwrap_or(0);
        let action = ctx.input_mut(|input| {
            navigation::take_grid_action(input, index, visible.len(), g.columns)
        });
        let next = match action {
            Some(navigation::GridAction::Select(next)) => Some(next),
            Some(navigation::GridAction::Sidebar) => {
                ctx.memory_mut(|memory| {
                    memory.move_focus(egui::FocusDirection::None);
                    memory.request_focus(nav_id(0));
                });
                return;
            }
            None => None,
        };
        if let Some(next) = next {
            let id = snapshot.games[visible[next]].id.clone();
            self.selected = Some(id.clone());
            self.scroll = g.reveal_scroll(next, visible.len(), self.scroll);
            ctx.memory_mut(|memory| {
                memory.move_focus(egui::FocusDirection::None);
                memory.request_focus(card_id(&id));
            });
        }
        let game = &snapshot.games[visible[next.unwrap_or(index)]];
        if ctx.input(|i| i.key_pressed(Key::Enter))
            && game.files_ok
            && !snapshot.running.contains(&game.id)
        {
            commands.push(AppCommand::Launch(game.id.clone()));
        }
        if ctx.input(|i| i.key_pressed(Key::F) && !i.modifiers.command && !i.modifiers.alt) {
            commands.push(AppCommand::ToggleFavorite(game.id.clone(), !game.favorite));
        }
    }

    fn games(
        &mut self,
        ui: &mut egui::Ui,
        drawing: PaintContext<'_>,
        snapshot: &AppSnapshot,
        visible: &[usize],
        enabled: bool,
        commands: &mut Vec<AppCommand>,
    ) {
        let p = drawing.painter;
        let g = drawing.geometry;
        let well = p.with_clip_rect(g.library);
        if visible.is_empty() {
            well.text(
                g.library.center() - vec2(0.0, 16.0),
                Align2::CENTER_CENTER,
                "NO GAMES IN THIS COLLECTION",
                FontId::monospace((24.0 * g.scale).max(12.0)),
                LIGHT,
            );
            well.text(
                g.library.center() + vec2(0.0, 13.0),
                Align2::CENTER_CENTER,
                if snapshot.refreshing {
                    "Reading the local library…"
                } else if self.game_sidebar.filter == sidebar::GameFilter::Html {
                    "Use IMPORT HTML / ZIP to add a downloaded local game."
                } else {
                    "Use SCAN INSTALLED GAMES, import a local HTML game, or choose another collection."
                },
                FontId::proportional((18.0 * g.scale).max(11.0)),
                MUTED,
            );
            return;
        }
        for (position, &i) in visible.iter().enumerate() {
            let game = &snapshot.games[i];
            let rect = g.card_rect(position, self.scroll);
            if !rect.intersects(g.library) {
                continue;
            }
            let response = ui.interact(
                rect.intersect(g.library),
                card_id(&game.id),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let selected = self.selected.as_ref() == Some(&game.id);
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &game.title)
            });
            let pointer_hit = ui
                .input(|input| input.pointer.interact_pos())
                .and_then(|point| g.card_at(point, visible.len(), self.scroll))
                == Some(position);
            if enabled && pointer_hit && response.clicked_by(PointerButton::Primary) {
                self.selected = Some(game.id.clone());
                response.request_focus();
                ui.ctx().request_repaint();
            }
            if enabled && activation::clicked_without_pointer(ui, &response) {
                self.selected = Some(game.id.clone());
                response.request_focus();
            }
            if enabled && response.has_focus() && self.selected.as_ref() != Some(&game.id) {
                self.selected = Some(game.id.clone());
                ui.ctx().request_repaint();
            }
            if enabled
                && pointer_hit
                && response.double_clicked()
                && game.files_ok
                && !snapshot.running.contains(&game.id)
            {
                commands.push(AppCommand::Launch(game.id.clone()));
            }
            self.card(
                PaintContext {
                    painter: &well,
                    geometry: g,
                },
                rect,
                game,
                selected,
                response.has_focus(),
                snapshot.running.contains(&game.id),
            );
            response.on_hover_text(format!(
                "{}\n{}\n{}",
                game.title,
                game.source,
                if !game.files_ok {
                    "Game files unavailable"
                } else if snapshot.running.contains(&game.id) {
                    activity(game)
                } else {
                    "Enter or double-click to launch · F to favorite"
                }
            ));
        }
        let max = g.max_scroll(visible.len());
        if max > 0.0 {
            let track = Rect::from_min_max(
                pos2(
                    g.library.right() - 5.0 * g.scale,
                    g.library.top() + 10.0 * g.scale,
                ),
                pos2(
                    g.library.right() - 2.0 * g.scale,
                    g.library.bottom() - 10.0 * g.scale,
                ),
            );
            let content_height = g.library.height() + max;
            let height = (track.height() * g.library.height() / content_height).max(16.0 * g.scale);
            let thumb = Rect::from_min_size(
                pos2(
                    track.left(),
                    track.top() + (track.height() - height) * self.scroll / max,
                ),
                vec2(track.width(), height),
            );
            well.rect_filled(track, 0.0, Color32::from_rgb(47, 20, 7));
            well.rect_filled(thumb, 0.0, AMBER.gamma_multiply(0.7));
            let response = ui.interact(
                thumb.expand(3.0 * g.scale),
                Id::new("pwds.scroll"),
                if enabled {
                    Sense::drag()
                } else {
                    Sense::hover()
                },
            );
            if response.dragged() {
                self.scroll = (self.scroll
                    + response.drag_delta().y * max / (track.height() - height).max(1.0))
                .clamp(0.0, max);
            }
        }
    }

    fn card(
        &self,
        drawing: PaintContext<'_>,
        rect: Rect,
        game: &GameCard,
        selected: bool,
        focused: bool,
        running: bool,
    ) {
        let p = drawing.painter;
        let g = drawing.geometry;
        let cut = g.card_cut();
        paint::cartridge(p, rect, cut, g.scale, selected);
        let art = rect.shrink(6.0 * g.scale);
        if let Some(texture) = self.covers.textures.get(&game.id) {
            let size = texture.handle.size();
            let ratio = size[0] as f32 / size[1] as f32;
            let target = art.width() / art.height();
            let uv = if ratio > target {
                let width = target / ratio;
                Rect::from_min_max(
                    pos2((1.0 - width) * 0.5, 0.0),
                    pos2((1.0 + width) * 0.5, 1.0),
                )
            } else {
                let height = ratio / target;
                Rect::from_min_max(
                    pos2(0.0, (1.0 - height) * 0.5),
                    pos2(1.0, (1.0 + height) * 0.5),
                )
            };
            paint::image(p, art, cut * 0.6, texture.handle.id(), uv, Color32::WHITE);
            let title =
                Rect::from_min_max(pos2(art.left(), art.top() + art.height() * 0.68), art.max);
            paint::polygon(
                p,
                &chamfer(title, [0.0, 0.0, cut * 0.6, cut * 0.6]),
                Color32::from_black_alpha(115),
                Color32::from_black_alpha(235),
            );
            let (title_text, caption) = card_label_regions(art, g.scale);
            let title_p = p.with_clip_rect(title_text);
            let galley = fonts::single_line(
                &title_p,
                &game.title.to_uppercase(),
                (18.0 * g.scale).max(10.0),
                title_text.width(),
                TEXT,
            );
            title_p.galley(
                title_text.left_center() - vec2(0.0, galley.size().y * 0.5),
                galley,
                TEXT,
            );
            let label = if game.origin == GameOrigin::Synthetic {
                "SYNTHETIC FIXTURE"
            } else if texture.synthetic {
                "GENERATED COVER"
            } else {
                game.source.as_str()
            };
            p.with_clip_rect(caption).text(
                caption.left_bottom(),
                Align2::LEFT_BOTTOM,
                label,
                FontId::monospace((10.0 * g.scale).max(7.0)),
                MUTED,
            );
        }
        if game.favorite {
            let badge = Rect::from_min_size(
                art.right_top() - vec2(26.0 * g.scale, 0.0),
                vec2(26.0, 24.0) * g.scale,
            );
            paint::polygon(
                p,
                &chamfer(badge, [0.0, 0.0, 5.0 * g.scale, 0.0]),
                AMBER,
                Color32::from_rgb(242, 106, 12),
            );
            star(
                p,
                badge.center(),
                7.0 * g.scale,
                Color32::from_rgb(25, 6, 0),
            );
        }
        if !game.files_ok || running {
            let status = if running {
                activity(game)
            } else {
                "FILES UNAVAILABLE"
            };
            let badge = Rect::from_min_max(art.min, pos2(art.right(), art.top() + 23.0 * g.scale));
            p.rect_filled(badge, 0.0, Color32::from_black_alpha(220));
            p.with_clip_rect(badge).text(
                badge.center(),
                Align2::CENTER_CENTER,
                status,
                FontId::monospace((11.0 * g.scale).max(8.0)),
                if running {
                    LIGHT
                } else {
                    Color32::from_rgb(255, 180, 168)
                },
            );
        }
        if focused {
            p.add(Shape::closed_line(
                chamfer(rect.shrink(1.0), [cut; 4]),
                Stroke::new(1.5 * g.scale.max(0.6), LIGHT),
            ));
        }
    }

    fn unavailable(&self, p: &Painter, g: &Geometry) {
        let p = p.with_clip_rect(g.library.shrink(18.0 * g.scale));
        let title = match self.section {
            1 => "SOUNDBOARD UNAVAILABLE",
            2 => "VOICE ROUTING UNAVAILABLE",
            _ => "MUSIC PLAYBACK UNAVAILABLE",
        };
        let at = g.library.center();
        p.text(
            at - vec2(0.0, 28.0 * g.scale),
            Align2::CENTER_CENTER,
            title,
            FontId::monospace((28.0 * g.scale).max(13.0)),
            LIGHT,
        );
        p.text(
            at + vec2(0.0, 7.0 * g.scale),
            Align2::CENTER_CENTER,
            "Audio is unavailable in this preview.",
            FontId::proportional((21.0 * g.scale).max(12.0)),
            MUTED,
        );
        p.text(
            at + vec2(0.0, 39.0 * g.scale),
            Align2::CENTER_CENTER,
            "Choose Games to return to your collection.",
            FontId::proportional((17.0 * g.scale).max(11.0)),
            MUTED,
        );
    }

    fn bottom(
        &self,
        ui: &mut egui::Ui,
        p: &Painter,
        g: &Geometry,
        snapshot: &AppSnapshot,
        enabled: bool,
        commands: &mut Vec<AppCommand>,
    ) {
        if self.section == 3 {
            let help = g.bottom.shrink2(vec2(12.0, 7.0) * g.scale);
            let text = p.with_clip_rect(help);
            text.text(
                help.left_top(),
                Align2::LEFT_TOP,
                format!(
                    "{} QUEUED  /  {} SAVED PLAYLISTS",
                    snapshot.music.queue.len(),
                    snapshot.music.playlists.len()
                ),
                FontId::monospace((20.0 * g.scale).max(11.0)),
                LIGHT,
            );
            text.text(
                help.left_bottom(),
                Align2::LEFT_BOTTOM,
                format!("QUEUE MANAGEMENT  /  {}", self.music.playback_status()),
                FontId::monospace((13.0 * g.scale).max(9.0)),
                MUTED,
            );
            self.problem_notice(ui, p, g, snapshot);
            return;
        }
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| snapshot.games.iter().find(|game| &game.id == id));
        let title_rect = Rect::from_min_max(
            g.bottom.min + vec2(3.0, 8.0) * g.scale,
            pos2(
                g.launch.left() - 30.0 * g.scale,
                g.bottom.bottom() - 7.0 * g.scale,
            ),
        );
        let text = p.with_clip_rect(title_rect);
        if self.section == 0 {
            if let Some(game) = selected {
                text.text(
                    title_rect.left_top(),
                    Align2::LEFT_TOP,
                    game.title.to_uppercase(),
                    fonts::control((34.0 * g.scale).max(14.0)),
                    Color32::from_rgb(255, 196, 122),
                );
                let status = if snapshot.running.contains(&game.id) {
                    activity(game)
                } else if game.files_ok {
                    "READY TO PLAY"
                } else {
                    "FILES UNAVAILABLE"
                };
                text.text(
                    title_rect.left_bottom(),
                    Align2::LEFT_BOTTOM,
                    format!("{}  /  {}", status, game.source.to_uppercase()),
                    FontId::monospace((13.0 * g.scale).max(9.0)),
                    MUTED,
                );
            } else {
                text.text(
                    title_rect.center(),
                    Align2::CENTER_CENTER,
                    "SELECT A GAME",
                    FontId::monospace((22.0 * g.scale).max(11.0)),
                    MUTED,
                );
            }
        } else {
            text.text(
                title_rect.center(),
                Align2::CENTER_CENTER,
                "AUDIO UNAVAILABLE",
                FontId::monospace((22.0 * g.scale).max(11.0)),
                MUTED,
            );
        }
        let can_launch = enabled
            && self.section == 0
            && selected.is_some_and(|game| game.files_ok && !snapshot.running.contains(&game.id));
        let launch = control(
            ui,
            p,
            g.launch,
            Id::new("pwds.launch"),
            "LAUNCH",
            ControlState {
                enabled: can_launch,
                selected: false,
            },
            7.0 * g.scale,
        );
        if activated(ui, &launch, g.launch, 7.0 * g.scale, can_launch)
            && let Some(game) = selected
        {
            commands.push(AppCommand::Launch(game.id.clone()));
        }
        let can_favorite = enabled && self.section == 0 && selected.is_some();
        let favorite = control(
            ui,
            p,
            g.favorite,
            Id::new("pwds.favorite"),
            if selected.is_some_and(|game| game.favorite) {
                "UNFAVORITE"
            } else {
                "FAVORITE"
            },
            ControlState {
                enabled: can_favorite,
                selected: false,
            },
            7.0 * g.scale,
        );
        if activated(ui, &favorite, g.favorite, 7.0 * g.scale, can_favorite)
            && let Some(game) = selected
        {
            commands.push(AppCommand::ToggleFavorite(game.id.clone(), !game.favorite));
        }
        let can_refresh =
            enabled && self.section == 0 && !snapshot.synthetic && !snapshot.refreshing;
        let refresh = control(
            ui,
            p,
            g.refresh,
            Id::new("pwds.refresh"),
            if snapshot.refreshing {
                "READING…"
            } else {
                "REFRESH"
            },
            ControlState {
                enabled: can_refresh,
                selected: false,
            },
            7.0 * g.scale,
        );
        if activated(ui, &refresh, g.refresh, 7.0 * g.scale, can_refresh) {
            commands.push(AppCommand::Refresh);
        }
        self.problem_notice(ui, p, g, snapshot);
    }

    fn problem_notice(&self, ui: &egui::Ui, p: &Painter, g: &Geometry, snapshot: &AppSnapshot) {
        if !snapshot.problems.is_empty() {
            let rect = Rect::from_min_size(
                pos2(
                    g.library.left() + 8.0 * g.scale,
                    g.library.bottom() - 24.0 * g.scale,
                ),
                vec2(g.library.width() - 16.0 * g.scale, 20.0 * g.scale),
            );
            p.rect_filled(rect, 0.0, Color32::from_black_alpha(225));
            p.with_clip_rect(rect).text(
                rect.left_center() + vec2(5.0 * g.scale, 0.0),
                Align2::LEFT_CENTER,
                format!(
                    "{} {} NOTICE(S): {}",
                    snapshot.problems.len(),
                    if self.section == 3 {
                        "MUSIC"
                    } else {
                        "LIBRARY"
                    },
                    snapshot.problems[0]
                ),
                FontId::monospace((12.0 * g.scale).max(9.0)),
                Color32::from_rgb(255, 180, 168),
            );
            ui.interact(rect, Id::new("pwds.problems"), Sense::hover())
                .on_hover_text(snapshot.problems.join("\n"));
        }
    }
}

fn nav_id(index: usize) -> Id {
    Id::new(("pwds.section", index))
}
fn card_id(id: &GameId) -> Id {
    Id::new(("pwds.card", format!("{id:?}")))
}

fn card_label_regions(art: Rect, scale: f32) -> (Rect, Rect) {
    let caption = Rect::from_min_max(
        pos2(
            art.left() + 5.0 * scale,
            art.bottom() - 6.0 * scale - (14.0 * scale).max(10.0),
        ),
        art.right_bottom() + vec2(-5.0, -6.0) * scale,
    );
    let title = Rect::from_min_max(
        pos2(
            art.left() + 5.0 * scale,
            art.top() + art.height() * 0.68 + 5.0 * scale,
        ),
        pos2(art.right() - 5.0 * scale, caption.top() - 3.0 * scale),
    );
    (title, caption)
}

fn control(
    ui: &egui::Ui,
    p: &Painter,
    rect: Rect,
    id: Id,
    label: &str,
    state: ControlState,
    cut: f32,
) -> Response {
    let ControlState { enabled, selected } = state;
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, label));
    let painter = paint::faded(p, if enabled { 1.0 } else { 0.43 });
    paint::slab(&painter, rect, cut, selected, response.has_focus());
    let label_rect = rect.shrink2(vec2(rect.width() * 0.12, 0.0));
    let center = rect.center()
        + vec2(
            if id == nav_id(0) || id == nav_id(1) || id == nav_id(2) || id == nav_id(3) {
                rect.width() * 0.05
            } else {
                0.0
            },
            0.0,
        );
    let width = 2.0 * (center.x - label_rect.left()).min(label_rect.right() - center.x);
    let color = if selected {
        Color32::from_rgb(22, 6, 0)
    } else {
        TEXT
    };
    let galley = fonts::fitted_control(
        &painter,
        label,
        (rect.height() * 0.48).max(11.0),
        width,
        color,
    );
    painter
        .with_clip_rect(label_rect)
        .galley(center - galley.size() * 0.5, galley, color);
    response
}

fn activity(game: &GameCard) -> &'static str {
    match &game.id {
        GameId::Store(_) => "OPENING",
        GameId::Html(_) => "RUNNING",
    }
}

fn glyph(p: &Painter, at: Pos2, radius: f32, index: usize, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    match index {
        0 => {
            for x in [-0.55, 0.55] {
                for y in [-0.55, 0.55] {
                    p.rect_stroke(
                        Rect::from_center_size(
                            at + vec2(x, y) * radius,
                            vec2(radius * 0.7, radius * 0.7),
                        ),
                        0.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        1 => {
            for (i, height) in [0.4, 0.9, 0.65, 1.0].iter().enumerate() {
                let x = at.x + (i as f32 - 1.5) * radius * 0.45;
                p.line_segment(
                    [
                        pos2(x, at.y - radius * height),
                        pos2(x, at.y + radius * height),
                    ],
                    stroke,
                );
            }
        }
        2 => {
            p.circle_stroke(at - vec2(0.0, radius * 0.3), radius * 0.42, stroke);
            p.line_segment([at, at + vec2(0.0, radius)], stroke);
            p.line_segment(
                [at + vec2(-0.5, 1.0) * radius, at + vec2(0.5, 1.0) * radius],
                stroke,
            );
        }
        _ => {
            p.line_segment(
                [
                    at + vec2(-0.3, 0.8) * radius,
                    at + vec2(-0.3, -0.8) * radius,
                ],
                stroke,
            );
            p.line_segment(
                [
                    at + vec2(-0.3, -0.8) * radius,
                    at + vec2(0.7, -0.45) * radius,
                ],
                stroke,
            );
            p.circle_filled(at + vec2(-0.6, 0.8) * radius, radius * 0.35, color);
        }
    }
}

fn star(p: &Painter, at: Pos2, radius: f32, color: Color32) {
    let points: Vec<_> = (0..10)
        .map(|i| {
            let a = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            at + vec2(a.cos(), a.sin()) * radius * if i % 2 == 0 { 1.0 } else { 0.45 }
        })
        .collect();
    paint::polygon(p, &points, color, color);
}

#[cfg(test)]
mod tests {
    use super::{card_label_regions, configure_context, fonts, geometry::Geometry, paint::TEXT};
    use eframe::egui::{Context, LayerId, RawInput, Rect, Theme, pos2, vec2};

    #[test]
    fn bundled_font_labels_fit_and_never_cover_fixture_provenance() {
        let ctx = Context::default();
        configure_context(&ctx);
        ctx.begin_pass(RawInput::default());
        let p = ctx.layer_painter(LayerId::background());
        for size in [vec2(880.0, 620.0), vec2(1250.0, 800.0), vec2(1672.0, 941.0)] {
            let g = Geometry::new(Rect::from_min_size(pos2(0.0, 0.0), size), 1.0);
            let filter = Rect::from_min_size(
                g.context.min,
                vec2(g.context.width(), (28.0 * g.scale).max(22.0)),
            );
            let width = filter.width() * 0.76;
            let label = fonts::fitted_control(
                &p,
                "RECENTLY PLAYED",
                (filter.height() * 0.48).max(11.0),
                width,
                TEXT,
            );
            assert!(label.size().x <= width + 0.01);
            assert!(!label.elided);
            let art = g.card_rect(0, 0.0).shrink(6.0 * g.scale);
            let (title, caption) = card_label_regions(art, g.scale);
            assert!(art.contains_rect(title) && art.contains_rect(caption));
            assert!(title.bottom() < caption.top());
            for name in [
                "STARBOUND ECHOES",
                "A VERY LONG TITLE THAT MUST NOT HIDE THE SOURCE LABEL",
            ] {
                let text =
                    fonts::single_line(&p, name, (18.0 * g.scale).max(10.0), title.width(), TEXT);
                assert_eq!(text.rows.len(), 1);
                assert!(text.size().x <= title.width() + 0.01);
                assert!(text.size().y <= title.height());
                if name == "STARBOUND ECHOES" {
                    assert!(!text.elided);
                }
            }
        }
        ctx.end_pass().drop_without_applying_deltas();
    }

    #[test]
    fn a_light_system_theme_cannot_replace_the_fixed_collection_popup_style() {
        let control = Context::default();
        control.begin_pass(RawInput {
            system_theme: Some(Theme::Light),
            ..Default::default()
        });
        assert!(!control.global_style().visuals.dark_mode);
        control.end_pass().drop_without_applying_deltas();

        let collection = Context::default();
        configure_context(&collection);
        collection.begin_pass(RawInput {
            system_theme: Some(Theme::Light),
            ..Default::default()
        });
        assert!(collection.global_style().visuals.dark_mode);
        collection.end_pass().drop_without_applying_deltas();
    }
}
