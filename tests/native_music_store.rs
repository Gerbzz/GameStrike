//! These fixtures use only owned temporary metadata and synthetic source bytes.
use gamestrike::music::{MusicCommand, MusicSnapshot, MusicStore, Track};
use serde_json::{Value, json};
use std::{fs, path::Path};
use uuid::Uuid;

fn paths(root: &Path, names: &[&str]) -> Vec<std::path::PathBuf> {
    names.iter().map(|name| root.join(name)).collect()
}

fn titles(tracks: &[Track]) -> Vec<&str> {
    tracks.iter().map(|track| track.title.as_str()).collect()
}

fn bytes(root: &Path) -> Vec<u8> {
    fs::read(root.join("music.json")).unwrap()
}

#[test]
fn missing_catalog_is_available_and_open_does_not_create_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("not-created");
    let store = MusicStore::open(&root).unwrap();
    let snapshot = store.snapshot();
    assert!(snapshot.available);
    assert!(snapshot.queue.is_empty() && snapshot.playlists.is_empty());
    assert_eq!(snapshot.problem, None);
    assert_eq!(snapshot.revision, 0);
    assert!(!root.exists());
    assert!(!MusicSnapshot::default().available);
}

#[test]
fn nonalphabetic_queue_order_move_remove_and_repeat_paths_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("catalog");
    let selected = paths(dir.path(), &["Zulu.wav", "Alpha.wav", "Middle.wav", "Zulu.wav"]);
    let mut store = MusicStore::open(&root).unwrap();
    let added = store.apply(MusicCommand::AddTracks(selected.clone())).unwrap();
    assert_eq!(titles(&added.queue), ["Zulu", "Alpha", "Middle", "Zulu"]);
    assert_ne!(added.queue[0].id, added.queue[3].id);
    assert_eq!(added.queue[0].path, added.queue[3].path);
    let middle = added.queue[2].id;
    store.apply(MusicCommand::MoveTrack { id: middle, up: true }).unwrap();
    let moved = store.apply(MusicCommand::MoveTrack { id: middle, up: false }).unwrap();
    assert_eq!(moved.queue, added.queue);
    store.apply(MusicCommand::MoveTrack { id: middle, up: true }).unwrap();
    let final_state = store.apply(MusicCommand::RemoveTrack(added.queue[3].id)).unwrap();
    assert_eq!(titles(&final_state.queue), ["Zulu", "Middle", "Alpha"]);
    assert_eq!(MusicStore::open(&root).unwrap().snapshot(), final_state);
    assert!(selected.iter().all(|path| !path.exists()), "paths are metadata, not files to create");
}

#[test]
fn two_saved_playlists_own_independent_ordered_track_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MusicStore::open(dir.path()).unwrap();
    let first = store.apply(MusicCommand::AddTracks(paths(dir.path(), &["Zulu.wav", "Alpha.wav", "Middle.wav"]))).unwrap();
    let first_saved = store.apply(MusicCommand::SavePlaylist { name: "  Original order  ".into() }).unwrap();
    assert_eq!(first_saved.playlists[0].name, "Original order");
    store.apply(MusicCommand::MoveTrack { id: first.queue[2].id, up: true }).unwrap();
    let saved = store.apply(MusicCommand::SavePlaylist { name: "Reordered".into() }).unwrap();
    assert_eq!(titles(&saved.playlists[0].tracks), ["Zulu", "Alpha", "Middle"]);
    assert_eq!(titles(&saved.playlists[1].tracks), ["Zulu", "Middle", "Alpha"]);
    store.apply(MusicCommand::RemoveTrack(first.queue[0].id)).unwrap();
    let restored = store.apply(MusicCommand::LoadPlaylist(saved.playlists[0].id)).unwrap();
    assert_eq!(restored.queue, saved.playlists[0].tracks);
    let restored = store.apply(MusicCommand::LoadPlaylist(saved.playlists[1].id)).unwrap();
    assert_eq!(restored.queue, saved.playlists[1].tracks);
    assert_eq!(restored.playlists, saved.playlists);
    assert_eq!(MusicStore::open(dir.path()).unwrap().snapshot(), restored);
}

#[test]
fn invalid_commands_preserve_snapshot_revision_and_catalog_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MusicStore::open(dir.path()).unwrap();
    store.apply(MusicCommand::AddTracks(paths(dir.path(), &["Zulu.wav", "Alpha.wav"]))).unwrap();
    let before = store.apply(MusicCommand::SavePlaylist { name: "My list".into() }).unwrap();
    let original = bytes(dir.path());
    let commands = vec![
        MusicCommand::AddTracks(vec![]),
        MusicCommand::AddTracks(vec![dir.path().join("valid.wav"), "relative.wav".into()]),
        MusicCommand::MoveTrack { id: Uuid::new_v4(), up: true },
        MusicCommand::MoveTrack { id: before.queue[0].id, up: true },
        MusicCommand::MoveTrack { id: before.queue[1].id, up: false },
        MusicCommand::RemoveTrack(Uuid::new_v4()),
        MusicCommand::LoadPlaylist(Uuid::new_v4()),
        MusicCommand::SavePlaylist { name: " ".into() },
        MusicCommand::SavePlaylist { name: "bad\nname".into() },
        MusicCommand::SavePlaylist { name: "x".repeat(81) },
        MusicCommand::SavePlaylist { name: " my LIST ".into() },
    ];
    for command in commands {
        assert!(store.apply(command).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(bytes(dir.path()), original);
    }
}

#[test]
fn queue_and_playlist_count_limits_refuse_without_partial_publication() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MusicStore::open(dir.path()).unwrap();
    let selected: Vec<_> = (0..256).map(|i| dir.path().join(format!("track-{i}.wav"))).collect();
    let before = store.apply(MusicCommand::AddTracks(selected)).unwrap();
    let original = bytes(dir.path());
    assert!(store.apply(MusicCommand::AddTracks(paths(dir.path(), &["overflow.wav"]))).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(bytes(dir.path()), original);

    let lists = dir.path().join("separate-lists");
    let mut store = MusicStore::open(&lists).unwrap();
    store.apply(MusicCommand::AddTracks(paths(dir.path(), &["one.wav"]))).unwrap();
    for i in 0..32 {
        store.apply(MusicCommand::SavePlaylist { name: format!("List {i}") }).unwrap();
    }
    let before = store.snapshot();
    let original = bytes(&lists);
    assert!(store.apply(MusicCommand::SavePlaylist { name: "Overflow".into() }).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(bytes(&lists), original);
}

#[test]
fn metadata_size_limit_is_enforced_before_a_new_state_is_published() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MusicStore::open(dir.path()).unwrap();
    let before = store.apply(MusicCommand::AddTracks(paths(dir.path(), &["one.wav"]))).unwrap();
    let original = bytes(dir.path());
    let prefix_length = dir.path().to_str().unwrap().len() + 1;
    let filename = format!("{}.wav", "x".repeat(4090 - prefix_length - 4));
    let long_path = dir.path().join(filename);
    let error = store.apply(MusicCommand::AddTracks(vec![long_path; 255])).unwrap_err();
    assert!(error.contains("metadata"), "{error}");
    assert_eq!(store.snapshot(), before);
    assert_eq!(bytes(dir.path()), original);
}

#[test]
fn failed_filesystem_write_keeps_memory_and_the_previous_catalog_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("catalog");
    let preserved = dir.path().join("preserved-catalog");
    let mut store = MusicStore::open(&root).unwrap();
    let before = store.apply(MusicCommand::AddTracks(paths(dir.path(), &["Zulu.wav"]))).unwrap();
    let original = bytes(&root);
    fs::rename(&root, &preserved).unwrap();
    fs::write(&root, b"blocks directory creation").unwrap();
    assert!(store.apply(MusicCommand::SavePlaylist { name: "Cannot write".into() }).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(bytes(&preserved), original);
    assert_eq!(fs::read(&root).unwrap(), b"blocks directory creation");
}

#[test]
fn corrupt_unknown_version_and_oversized_catalogs_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    for invalid in [b"not JSON".to_vec(), br#"{"schema_version":99,"revision":0,"queue":[],"playlists":[]}"#.to_vec(), vec![b' '; 1024 * 1024 + 1]] {
        fs::write(dir.path().join("music.json"), &invalid).unwrap();
        assert!(MusicStore::open(dir.path()).is_err());
        assert_eq!(bytes(dir.path()), invalid);
    }
}

#[test]
fn deserialized_ids_paths_names_and_collection_limits_are_validated() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MusicStore::open(dir.path()).unwrap();
    store.apply(MusicCommand::AddTracks(paths(dir.path(), &["Zulu.wav"]))).unwrap();
    store.apply(MusicCommand::SavePlaylist { name: "Saved".into() }).unwrap();
    let valid: Value = serde_json::from_slice(&bytes(dir.path())).unwrap();
    let mut invalid = Vec::new();
    let mut value = valid.clone();
    value["queue"][0]["id"] = json!(Uuid::nil());
    invalid.push(value);
    let mut value = valid.clone();
    value["queue"][0]["path"] = json!("relative.wav");
    invalid.push(value);
    let mut value = valid.clone();
    value["queue"][0]["title"] = json!("bad\nname");
    invalid.push(value);
    let mut value = valid.clone();
    value["queue"][0]["title"] = json!("x".repeat(81));
    invalid.push(value);
    let mut value = valid.clone();
    value["queue"][0]["path"] = json!(format!("/{}", "x".repeat(4096)));
    invalid.push(value);
    let mut value = valid.clone();
    value["queue"] = json!([valid["queue"][0].clone(), valid["queue"][0].clone()]);
    invalid.push(value);
    let mut value = valid.clone();
    value["queue"] = json!(vec![valid["queue"][0].clone(); 257]);
    invalid.push(value);
    let mut value = valid.clone();
    value["playlists"][0]["id"] = json!(Uuid::nil());
    invalid.push(value);
    let mut value = valid.clone();
    value["playlists"][0]["name"] = json!(" untrimmed ");
    invalid.push(value);
    let mut value = valid.clone();
    value["playlists"] = json!(vec![valid["playlists"][0].clone(); 33]);
    invalid.push(value);
    let mut value = valid.clone();
    value["playlists"][0]["tracks"][0]["id"] = json!(Uuid::nil());
    invalid.push(value);
    let mut value = valid.clone();
    value["playlists"][0]["tracks"] = json!(vec![valid["queue"][0].clone(); 257]);
    invalid.push(value);
    let mut duplicate = valid["playlists"][0].clone();
    duplicate["name"] = json!("Different name, same ID");
    let mut value = valid.clone();
    value["playlists"] = json!([valid["playlists"][0].clone(), duplicate]);
    invalid.push(value);
    let mut duplicate = valid["playlists"][0].clone();
    duplicate["id"] = json!(Uuid::new_v4());
    duplicate["name"] = json!("SAVED");
    let mut value = valid.clone();
    value["playlists"] = json!([valid["playlists"][0].clone(), duplicate]);
    invalid.push(value);
    let mut value = valid;
    value["unexpected"] = json!(true);
    invalid.push(value);
    for value in invalid {
        let original = serde_json::to_vec(&value).unwrap();
        fs::write(dir.path().join("music.json"), &original).unwrap();
        assert!(MusicStore::open(dir.path()).is_err());
        assert_eq!(bytes(dir.path()), original);
    }
}

#[test]
fn unreadable_catalog_kind_and_external_changes_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("directory-catalog");
    fs::create_dir_all(root.join("music.json")).unwrap();
    assert!(MusicStore::open(&root).is_err());

    let root = dir.path().join("changed-catalog");
    let mut store = MusicStore::open(&root).unwrap();
    let before = store.apply(MusicCommand::AddTracks(paths(dir.path(), &["one.wav"]))).unwrap();
    let external = b"externally corrupted metadata";
    fs::write(root.join("music.json"), external).unwrap();
    assert!(store.apply(MusicCommand::SavePlaylist { name: "Do not overwrite".into() }).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(bytes(&root), external);
}

#[test]
fn source_files_are_never_changed_copied_deleted_or_required_to_exist() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("selected.wav");
    let original = b"owned synthetic bytes; not decoded as media";
    fs::write(&source, original).unwrap();
    let root = dir.path().join("metadata-only");
    let mut store = MusicStore::open(&root).unwrap();
    let added = store.apply(MusicCommand::AddTracks(vec![source.clone()])).unwrap();
    let saved = store.apply(MusicCommand::SavePlaylist { name: "Keep reference".into() }).unwrap();
    store.apply(MusicCommand::RemoveTrack(added.queue[0].id)).unwrap();
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1, "only music.json is stored");
    fs::remove_file(&source).unwrap();
    let restored = store.apply(MusicCommand::LoadPlaylist(saved.playlists[0].id)).unwrap();
    assert_eq!(restored.queue[0].path, source);
    assert!(!source.exists());
    assert_eq!(MusicStore::open(&root).unwrap().snapshot(), restored);
}
