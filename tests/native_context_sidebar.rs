#![cfg(feature = "native-ui")]

use eframe::egui::{
    Context, Event, Id, Key, Modifiers, PointerButton, RawInput, Rect, Ui, UiBuilder, pos2, vec2,
};
use gamestrike::{
    application::{AppCommand, AppSnapshot, GameCard, GameId, GameOrigin, LauncherService},
    music::{MusicCommand, MusicSnapshot, Playlist},
    native_ui::{
        configure_context,
        geometry::Geometry,
        music::MusicPanel,
        sidebar::{GameFilter, GamesSidebar},
    },
    stores::{Store, StoreStatus},
};
use uuid::Uuid;

fn game(number: u128, origin: GameOrigin, source: &str) -> GameCard {
    GameCard {
        id: match origin {
            GameOrigin::Store(_) => GameId::Store(number.to_string()),
            _ => GameId::Html(Uuid::from_u128(number)),
        },
        origin,
        title: format!("Owned test game {number}"),
        source: source.into(),
        favorite: number == 1,
        files_ok: true,
        last_played: (number < 3).then_some(number as u64),
        cover: None,
    }
}
fn snapshot() -> AppSnapshot {
    AppSnapshot {
        games: vec![
            game(1, GameOrigin::Store(Store::Steam), "Steam"),
            game(2, GameOrigin::Html, "Steam"),
            game(3, GameOrigin::Store(Store::Ubisoft), "Ubisoft Connect"),
            game(4, GameOrigin::Store(Store::Custom), "Custom"),
            game(5, GameOrigin::Synthetic, "OWNED SYNTHETIC FIXTURE"),
        ],
        providers: Store::ALL
            .into_iter()
            .map(|store| StoreStatus {
                key: store.key(),
                name: store.name(),
                enabled: true,
                supported: true,
                games: 999,
            })
            .collect(),
        music: MusicSnapshot {
            available: true,
            playlists: vec![Playlist {
                id: Uuid::from_u128(30),
                name: "Owned playlist fixture".into(),
                tracks: vec![],
            }],
            ..Default::default()
        },
        ..Default::default()
    }
}

struct Harness {
    ctx: Context,
    games: GamesSidebar,
    music: MusicPanel,
    section: usize,
    size: [f32; 2],
    time: f64,
    clip: Option<Rect>,
}
impl Harness {
    fn new(size: [f32; 2]) -> Self {
        let ctx = Context::default();
        configure_context(&ctx);
        Self {
            ctx,
            games: GamesSidebar::default(),
            music: MusicPanel::default(),
            section: 0,
            size,
            time: 0.0,
            clip: None,
        }
    }
    fn draw(
        &mut self,
        snapshot: &AppSnapshot,
        events: Vec<Event>,
        enabled: bool,
    ) -> Vec<AppCommand> {
        self.time += 0.1;
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(self.size[0], self.size[1]));
        let g = Geometry::new(viewport, 1.0);
        self.ctx.begin_pass(RawInput {
            screen_rect: Some(viewport),
            time: Some(self.time),
            focused: true,
            events,
            ..Default::default()
        });
        let mut ui = Ui::new(
            self.ctx.clone(),
            Id::new("context.test.root"),
            UiBuilder::new().max_rect(viewport),
        );
        let mut commands = ui
            .scope_builder(UiBuilder::new().max_rect(g.context), |ui| {
                ui.set_clip_rect(
                    self.clip
                        .map_or(g.context, |clip| clip.intersect(g.context)),
                );
                if self.section == 0 {
                    self.games.show(ui, g.context, g.scale, snapshot, enabled)
                } else {
                    self.music.show_sidebar(
                        ui,
                        g.context,
                        g.scale,
                        &snapshot.music,
                        enabled && !snapshot.synthetic,
                    )
                }
            })
            .inner;
        if self.section == 3 {
            commands.extend(
                self.music
                    .show(&mut ui, g.library, g.scale, &snapshot.music, enabled),
            );
        }
        drop(ui);
        self.ctx.end_pass().drop_without_applying_deltas();
        commands
    }
    fn click(&mut self, snapshot: &AppSnapshot, id: Id, enabled: bool) -> Vec<AppCommand> {
        let point = self
            .ctx
            .read_response(id)
            .expect("actual visible control")
            .rect
            .center();
        assert!(
            self.draw(
                snapshot,
                vec![
                    Event::PointerMoved(point),
                    Event::PointerButton {
                        pos: point,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE
                    }
                ],
                enabled
            )
            .is_empty()
        );
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
    fn key(&mut self, snapshot: &AppSnapshot, id: Id, key: Key, enabled: bool) -> Vec<AppCommand> {
        self.ctx.memory_mut(|memory| memory.request_focus(id));
        self.draw(
            snapshot,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            enabled,
        )
    }
}

#[test]
fn filters_use_typed_origins_not_html_metadata_and_keep_recent_order() {
    let snapshot = snapshot();
    let mut sidebar = GamesSidebar::default();
    for (filter, expected) in [
        (GameFilter::All, vec![0, 1, 2, 3, 4]),
        (GameFilter::Favorites, vec![0]),
        (GameFilter::Recent, vec![1, 0]),
        (GameFilter::Provider(Store::Steam), vec![0]),
        (GameFilter::Provider(Store::Ubisoft), vec![2]),
        (GameFilter::Provider(Store::Custom), vec![3]),
        (GameFilter::Html, vec![1]),
    ] {
        sidebar.filter = filter;
        assert_eq!(sidebar.visible(&snapshot), expected);
    }
    let synthetic = LauncherService::synthetic();
    sidebar.filter = GameFilter::Html;
    assert!(sidebar.visible(synthetic.snapshot()).is_empty());
    for store in Store::ALL {
        sidebar.filter = GameFilter::Provider(store);
        assert!(sidebar.visible(synthetic.snapshot()).is_empty());
    }
}

#[test]
fn provider_browsing_and_html_filter_emit_no_host_intent_at_supported_sizes() {
    for size in [[880.0, 620.0], [1250.0, 800.0], [1672.0, 941.0]] {
        let mut h = Harness::new(size);
        let s = snapshot();
        h.draw(&s, vec![], true);
        for store in Store::ALL {
            // Keyboard focus reveals clipped rows through the same production ScrollArea.
            let id = Id::new(("pwds.games.provider", store.key()));
            assert!(h.key(&s, id, Key::Enter, true).is_empty());
            assert_eq!(h.games.filter, GameFilter::Provider(store));
            for _ in 0..10 {
                h.draw(&s, vec![], true);
            }
            let revealed = h.ctx.read_response(id).unwrap();
            assert!(
                revealed.interact_rect.contains(revealed.rect.center()),
                "{store:?} must be visible after keyboard focus at {size:?}"
            );
        }
        assert!(h.click(&s, Id::new("pwds.games.html"), true).is_empty());
        assert_eq!(h.games.filter, GameFilter::Html);
        let g = Geometry::new(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(size[0], size[1])),
            1.0,
        );
        for id in ["pwds.games.html", "pwds.games.import", "pwds.games.scan"] {
            assert!(
                g.context
                    .contains_rect(h.ctx.read_response(Id::new(id)).unwrap().rect)
            );
        }
    }
}

#[test]
fn visible_import_and_scan_are_explicit_separate_intents_and_disabled_in_capture_or_boot() {
    let mut h = Harness::new([1250.0, 800.0]);
    let mut s = snapshot();
    h.draw(&s, vec![], true);
    let commands = h.click(&s, Id::new("pwds.games.import"), true);
    assert!(matches!(commands.as_slice(), [AppCommand::ChooseHtmlFile]));
    assert_eq!(h.games.filter, GameFilter::Html);
    let commands = h.click(&s, Id::new("pwds.games.scan"), true);
    assert!(matches!(commands.as_slice(), [AppCommand::Refresh]));
    s.refreshing = true;
    h.draw(&s, vec![], true);
    assert!(h.click(&s, Id::new("pwds.games.scan"), true).is_empty());
    for synthetic in [false, true] {
        s.synthetic = synthetic;
        let enabled = synthetic; // false=boot gated, true=synthetic capture gated
        h.draw(&s, vec![], enabled);
        for id in ["pwds.games.import", "pwds.games.scan"] {
            assert!(h.click(&s, Id::new(id), enabled).is_empty());
        }
    }
}

#[test]
fn unavailable_scanners_do_not_fake_support_but_retained_catalog_entries_remain_browsable() {
    let mut h = Harness::new([1250.0, 800.0]);
    let mut s = snapshot();
    s.providers
        .iter_mut()
        .find(|s| s.key == "epic")
        .unwrap()
        .supported = false;
    s.providers
        .iter_mut()
        .find(|s| s.key == "ea")
        .unwrap()
        .enabled = false;
    s.providers
        .iter_mut()
        .find(|s| s.key == "ubisoft")
        .unwrap()
        .supported = false;
    h.draw(&s, vec![], true);
    for store in [Store::Epic, Store::Ea] {
        assert!(
            h.key(
                &s,
                Id::new(("pwds.games.provider", store.key())),
                Key::Enter,
                true
            )
            .is_empty()
        );
        assert_eq!(h.games.filter, GameFilter::All);
        h.draw(&s, vec![], true);
    }
    assert!(
        h.key(
            &s,
            Id::new(("pwds.games.provider", "ubisoft")),
            Key::Enter,
            true
        )
        .is_empty()
    );
    assert_eq!(h.games.filter, GameFilter::Provider(Store::Ubisoft));
}

#[test]
fn music_context_selection_is_retained_and_loading_is_explicit_not_autoplay() {
    let mut h = Harness::new([1250.0, 800.0]);
    let s = snapshot();
    h.games.filter = GameFilter::Provider(Store::Steam);
    h.section = 3;
    h.draw(&s, vec![], true);
    assert!(
        h.click(
            &s,
            Id::new(("pwds.music.sidebar.playlist", Uuid::from_u128(30))),
            true
        )
        .is_empty()
    );
    h.section = 0;
    h.draw(&s, vec![], true);
    assert_eq!(h.games.filter, GameFilter::Provider(Store::Steam));
    h.section = 3;
    h.draw(&s, vec![], true);
    let commands = h.click(&s, Id::new("pwds.music.sidebar.load"), true);
    assert!(
        matches!(commands.as_slice(),[AppCommand::Music(MusicCommand::LoadPlaylist(id))] if *id==Uuid::from_u128(30))
    );
    let commands = h.click(&s, Id::new("pwds.music.sidebar.import"), true);
    assert!(matches!(
        commands.as_slice(),
        [AppCommand::ChooseMusicFiles]
    ));
}

#[test]
fn current_queue_keyboard_navigation_cannot_activate_the_main_panel_file_picker() {
    for key in [Key::Enter, Key::Space] {
        let mut h = Harness::new([1250.0, 800.0]);
        h.section = 3;
        let s = snapshot();
        h.draw(&s, vec![], true);
        assert!(
            h.key(&s, Id::new("pwds.music.sidebar.queue"), key, true)
                .is_empty()
        );
        assert_eq!(
            h.ctx.memory(|memory| memory.focused()),
            Some(Id::new("pwds.music.sidebar.queue"))
        );
    }
}

#[test]
fn deleted_playlist_cannot_load_a_stale_selection() {
    let mut h = Harness::new([1250.0, 800.0]);
    h.section = 3;
    let mut s = snapshot();
    h.draw(&s, vec![], true);
    h.click(
        &s,
        Id::new(("pwds.music.sidebar.playlist", Uuid::from_u128(30))),
        true,
    );
    s.music.playlists.clear();
    h.draw(&s, vec![], true);
    assert!(
        h.click(&s, Id::new("pwds.music.sidebar.load"), true)
            .is_empty()
    );
}

#[test]
fn a_pointer_in_a_clipped_provider_row_cannot_change_the_filter() {
    let mut h = Harness::new([880.0, 620.0]);
    let s = snapshot();
    h.draw(&s, vec![], true);
    let id = Id::new(("pwds.games.provider", "steam"));
    let row = h.ctx.read_response(id).unwrap().rect;
    h.clip = Some(Rect::from_min_max(
        pos2(0.0, 0.0),
        pos2(880.0, row.top() + row.height() * 0.25),
    ));
    h.draw(&s, vec![], true);
    assert!(h.click(&s, id, true).is_empty());
    assert_eq!(h.games.filter, GameFilter::All);
}

#[test]
fn music_context_host_actions_are_disabled_when_storage_is_unavailable_or_boot_is_blocked() {
    for available in [false, true] {
        let mut h = Harness::new([1250.0, 800.0]);
        h.section = 3;
        let mut s = snapshot();
        s.music.available = available;
        let enabled = !available;
        h.draw(&s, vec![], enabled);
        for id in ["pwds.music.sidebar.import", "pwds.music.sidebar.load"] {
            assert!(h.click(&s, Id::new(id), enabled).is_empty());
        }
    }
}
