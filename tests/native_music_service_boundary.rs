use gamestrike::{
    application::{AppCommand, DesktopHost, LauncherService, LaunchOutcome, LibraryBackend},
    music::{MusicCommand, playback::PlaybackCommand},
};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct NoDeviceHost;

impl DesktopHost for NoDeviceHost {
    fn refresh_stores(&mut self, _: &gamestrike::stores::StoreLibrary) -> Result<(), String> {
        panic!("music transport must not refresh game stores")
    }

    fn launch_store(&mut self, _: &gamestrike::stores::Launch) -> Result<LaunchOutcome, String> {
        panic!("music transport must not launch a game")
    }

    fn launch_html(&mut self, _: &Path, _: Uuid) -> Result<LaunchOutcome, String> {
        panic!("music transport must not launch a player")
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
            "catalog worker did not publish its music result"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn only_successful_playlist_load_marks_committed_queue_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let backend = LibraryBackend::open(root.clone(), NoDeviceHost).unwrap();
    let mut service = LauncherService::start(move || Ok(Box::new(backend)));
    until(&mut service, |service| service.snapshot().music.available);
    assert_eq!(service.music_replacement_revision(), 0);
    service
        .dispatch(AppCommand::Music(MusicCommand::AddTracks(vec![
            root.join("Owned.wav"),
        ])))
        .unwrap();
    until(&mut service, |service| service.snapshot().music.queue.len() == 1);
    service
        .dispatch(AppCommand::Music(MusicCommand::SavePlaylist {
            name: "Saved".into(),
        }))
        .unwrap();
    until(&mut service, |service| service.snapshot().music.playlists.len() == 1);
    assert_eq!(service.music_replacement_revision(), 0);
    let playlist = service.snapshot().music.playlists[0].id;
    service
        .dispatch(AppCommand::Music(MusicCommand::LoadPlaylist(playlist)))
        .unwrap();
    until(&mut service, |service| service.music_replacement_revision() == 1);
    let committed = service.snapshot().music.clone();
    service
        .dispatch(AppCommand::Music(MusicCommand::LoadPlaylist(Uuid::nil())))
        .unwrap();
    until(&mut service, |service| !service.snapshot().problems.is_empty());
    assert_eq!(service.music_replacement_revision(), 1);
    assert_eq!(service.snapshot().music, committed);
    assert!(!root.join("Owned.wav").exists());
}

#[test]
fn catalog_and_synthetic_services_reject_playback_intents() {
    let temp = tempfile::tempdir().unwrap();
    let backend = LibraryBackend::open(temp.path().to_path_buf(), NoDeviceHost).unwrap();
    let mut service = LauncherService::start(move || Ok(Box::new(backend)));
    until(&mut service, |service| service.snapshot().music.available);
    for command in [PlaybackCommand::Play(Uuid::nil()), PlaybackCommand::Stop] {
        assert!(
            service
                .dispatch(AppCommand::Playback(command.clone()))
                .is_err()
        );
        assert!(
            LauncherService::synthetic()
                .dispatch(AppCommand::Playback(command))
                .is_err()
        );
    }
    assert!(service.snapshot().music.queue.is_empty());
}
