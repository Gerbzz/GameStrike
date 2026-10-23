use gamestrike::{
    Library,
    application::{CatalogBackend, DesktopHost, GameId, LaunchOutcome, LibraryBackend},
    stores::{Found, Launch, Store, StoreLibrary, store_game_id},
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use uuid::Uuid;

struct FakeHost(Arc<AtomicUsize>);
impl DesktopHost for FakeHost {
    fn refresh_stores(&mut self, _stores: &StoreLibrary) -> Result<(), String> {
        Ok(())
    }
    fn launch_store(&mut self, _target: &Launch) -> Result<LaunchOutcome, String> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(LaunchOutcome::HandedOff)
    }
    fn launch_html(&mut self, _root: &Path, _id: Uuid) -> Result<LaunchOutcome, String> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(LaunchOutcome::HandedOff)
    }
}

#[test]
fn fallible_native_catalog_read_does_not_mask_corruption_as_an_empty_library() {
    let temp = tempfile::tempdir().unwrap();
    Library::open(temp.path()).unwrap();
    let stores = StoreLibrary::open(temp.path()).unwrap();
    let path = temp.path().join("stores/catalog.json");
    let original = b"{broken persisted catalog";
    fs::write(&path, original).unwrap();
    assert!(stores.try_entries().is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn absent_catalog_is_a_valid_empty_native_collection() {
    let temp = tempfile::tempdir().unwrap();
    let stores = StoreLibrary::open(temp.path()).unwrap();
    assert!(stores.try_entries().unwrap().is_empty());
}

#[test]
fn native_backend_reuses_import_ids_and_persists_favorites_across_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let source = temp.path().join("owned.html");
    fs::write(&source, "<!doctype html><title>Owned fixture</title>").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut backend = LibraryBackend::open(root.clone(), FakeHost(Arc::clone(&calls))).unwrap();
    backend.import(&source).unwrap();
    let first = backend.snapshot().unwrap();
    assert_eq!(first.games.len(), 1);
    let id = first.games[0].id.clone();
    backend.set_favorite(&id, true).unwrap();
    backend.import(&source).unwrap();
    drop(backend);
    let mut reopened = LibraryBackend::open(root, FakeHost(Arc::clone(&calls))).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.games.len(), 1);
    assert_eq!(snapshot.games[0].id, id);
    assert!(snapshot.games[0].favorite);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn forged_store_uri_is_rejected_before_reaching_the_native_host() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut backend =
        LibraryBackend::open(temp.path().to_path_buf(), FakeHost(Arc::clone(&calls))).unwrap();
    let id = store_game_id(Store::Steam, "123");
    let found = Found {
        store: Store::Steam,
        store_id: "123".into(),
        title: "Owned malicious-target fixture".into(),
        install_path: None,
        cover_source: None,
        launch: Launch::Uri {
            uri: "https://example.invalid/not-a-game-launch".into(),
        },
    };
    let catalog =
        serde_json::json!({"games": [{"id": id,"found": found,"installed": true,"first_seen": 0}]});
    fs::write(
        temp.path().join("stores/catalog.json"),
        serde_json::to_vec(&catalog).unwrap(),
    )
    .unwrap();
    assert!(backend.launch(&GameId::Store(id.to_string())).is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
