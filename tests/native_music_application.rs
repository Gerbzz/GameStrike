//! Synthetic persistence and application-service regressions.
use gamestrike::{
    application::{AppCommand, AppSnapshot, CatalogBackend, LaunchOutcome, LauncherService},
    music::{MusicCommand, MusicSnapshot, MusicStore},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct MusicBackend {
    store: MusicStore,
    scans: Arc<AtomicUsize>,
}

impl CatalogBackend for MusicBackend {
    fn snapshot(&mut self) -> Result<AppSnapshot, String> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        Ok(AppSnapshot {
            music: self.store.snapshot(),
            ..Default::default()
        })
    }
    fn update_music(&mut self, command: MusicCommand) -> Result<MusicSnapshot, String> {
        self.store.apply(command)
    }
    fn refresh(&mut self) -> Result<(), String> {
        panic!("music must not scan games")
    }
    fn import(&mut self, _: &Path) -> Result<(), String> {
        panic!("music must not import games")
    }
    fn set_favorite(&mut self, _: &gamestrike::application::GameId, _: bool) -> Result<(), String> {
        panic!("music must not edit game metadata")
    }
    fn launch(&mut self, _: &gamestrike::application::GameId) -> Result<LaunchOutcome, String> {
        panic!("queue edits must not launch a game or player")
    }
}

fn until(service: &mut LauncherService, predicate: impl Fn(&LauncherService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        service.poll_events();
        if predicate(service) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "music worker did not publish a result"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn music_commands_publish_committed_order_without_rescanning_games() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let scans = Arc::new(AtomicUsize::new(0));
    let wakes = Arc::new(AtomicUsize::new(0));
    let store = MusicStore::open(&root).unwrap();
    let backend = MusicBackend {
        store,
        scans: scans.clone(),
    };
    let notify = wakes.clone();
    let mut service = LauncherService::start_with_waker(
        move || Ok(Box::new(backend)),
        Arc::new(move || {
            notify.fetch_add(1, Ordering::SeqCst);
        }),
    );
    until(&mut service, |s| s.snapshot().music.available);
    let catalog_revision = service.snapshot().revision;
    let wake_before = wakes.load(Ordering::SeqCst);
    service
        .dispatch(AppCommand::Music(MusicCommand::AddTracks(vec![
            root.join("Zeta.wav"),
            root.join("Alpha.wav"),
            root.join("Middle.wav"),
        ])))
        .unwrap();
    assert!(
        service.snapshot().music.queue.is_empty(),
        "no optimistic unsaved queue"
    );
    until(&mut service, |s| s.snapshot().music.queue.len() == 3);
    let middle = service.snapshot().music.queue[2].id;
    service
        .dispatch(AppCommand::Music(MusicCommand::MoveTrack {
            id: middle,
            up: true,
        }))
        .unwrap();
    until(&mut service, |s| s.snapshot().music.queue[1].id == middle);
    service
        .dispatch(AppCommand::Music(MusicCommand::SavePlaylist {
            name: "Evening".into(),
        }))
        .unwrap();
    until(&mut service, |s| s.snapshot().music.playlists.len() == 1);
    let committed = service.snapshot().music.clone();
    assert_eq!(
        scans.load(Ordering::SeqCst),
        1,
        "music mutations must not reload game covers/catalogs"
    );
    assert_eq!(
        service.snapshot().revision,
        catalog_revision,
        "music must not invalidate game cover textures"
    );
    assert!(
        wakes.load(Ordering::SeqCst) > wake_before,
        "worker must wake an idle native UI"
    );
    let reopened = MusicStore::open(&root).unwrap().snapshot();
    assert_eq!(reopened.queue, committed.queue);
    assert_eq!(reopened.playlists, committed.playlists);
    assert!(
        !root.join("Zeta.wav").exists(),
        "metadata cannot create/copy music files"
    );
}

#[test]
fn failed_persistence_retains_the_last_committed_native_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let mut store = MusicStore::open(&root).unwrap();
    store
        .apply(MusicCommand::AddTracks(vec![root.join("Owned.wav")]))
        .unwrap();
    let backend = MusicBackend {
        store,
        scans: Arc::new(AtomicUsize::new(0)),
    };
    let mut service = LauncherService::start(move || Ok(Box::new(backend)));
    until(&mut service, |s| s.snapshot().music.queue.len() == 1);
    let before = service.snapshot().music.clone();
    // A directory at the exact metadata destination forces a real save failure on every OS.
    std::fs::remove_file(root.join("music.json")).unwrap();
    std::fs::create_dir(root.join("music.json")).unwrap();
    service
        .dispatch(AppCommand::Music(MusicCommand::SavePlaylist {
            name: "Cannot save".into(),
        }))
        .unwrap();
    until(&mut service, |s| !s.snapshot().problems.is_empty());
    assert_eq!(service.snapshot().music, before);
    assert!(root.join("music.json").is_dir());
}

#[test]
fn synthetic_service_and_worker_have_no_file_picker_authority() {
    let mut synthetic = LauncherService::synthetic();
    assert!(synthetic.dispatch(AppCommand::ChooseMusicFiles).is_err());
    assert!(
        synthetic
            .dispatch(AppCommand::Music(MusicCommand::SavePlaylist {
                name: "Forbidden".into(),
            }))
            .is_err()
    );
    let temp = tempfile::tempdir().unwrap();
    let backend = MusicBackend {
        store: MusicStore::open(temp.path()).unwrap(),
        scans: Arc::new(AtomicUsize::new(0)),
    };
    let mut service = LauncherService::start(move || Ok(Box::new(backend)));
    until(&mut service, |s| s.snapshot().music.available);
    assert!(service.dispatch(AppCommand::ChooseMusicFiles).is_err());
    assert!(service.snapshot().music.queue.is_empty());
}

struct NoDeviceHost;
impl gamestrike::application::DesktopHost for NoDeviceHost {
    fn refresh_stores(&mut self, _: &gamestrike::stores::StoreLibrary) -> Result<(), String> {
        panic!("music queue must not invoke a store scan")
    }
    fn launch_store(&mut self, _: &gamestrike::stores::Launch) -> Result<LaunchOutcome, String> {
        panic!("music queue must not invoke an OS launcher")
    }
    fn launch_html(&mut self, _: &Path, _: uuid::Uuid) -> Result<LaunchOutcome, String> {
        panic!("music queue must not invoke a player")
    }
}

#[test]
fn real_library_backend_connects_persisted_queue_and_saved_playlists() {
    use gamestrike::application::LibraryBackend;
    let temp = tempfile::tempdir().unwrap();
    let mut backend = LibraryBackend::open(temp.path().to_path_buf(), NoDeviceHost).unwrap();
    assert!(backend.snapshot().unwrap().music.available);
    let added = backend
        .update_music(MusicCommand::AddTracks(vec![
            temp.path().join("Second.wav"),
            temp.path().join("First.wav"),
        ]))
        .unwrap();
    let saved = backend
        .update_music(MusicCommand::SavePlaylist {
            name: "Original order".into(),
        })
        .unwrap();
    assert_eq!(saved.playlists[0].tracks, added.queue);
    drop(backend);
    let mut reopened = LibraryBackend::open(temp.path().to_path_buf(), NoDeviceHost).unwrap();
    let reopened = reopened.snapshot().unwrap().music;
    assert_eq!(reopened.queue, saved.queue);
    assert_eq!(reopened.playlists, saved.playlists);
}

#[test]
fn corrupt_music_metadata_disables_music_without_disabling_games_or_overwriting_it() {
    use gamestrike::application::LibraryBackend;
    let temp = tempfile::tempdir().unwrap();
    let invalid = b"{invalid music queue";
    std::fs::write(temp.path().join("music.json"), invalid).unwrap();
    let mut backend = LibraryBackend::open(temp.path().to_path_buf(), NoDeviceHost).unwrap();
    let snapshot = backend.snapshot().unwrap();
    assert!(!snapshot.music.available);
    assert!(snapshot.music.problem.is_some());
    assert!(
        backend
            .update_music(MusicCommand::AddTracks(vec![
                temp.path().join("Ignored.wav")
            ]))
            .is_err()
    );
    assert_eq!(
        std::fs::read(temp.path().join("music.json")).unwrap(),
        invalid
    );
}
