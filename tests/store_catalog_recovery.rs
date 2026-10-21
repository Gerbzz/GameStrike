use gamestrike::{
    HostOs,
    stores::{NoRegistry, ScanEnv, Store, StoreLibrary},
};
use std::fs;

fn damaged_catalog_is_preserved(bytes: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let library = StoreLibrary::open(dir.path()).unwrap();
    let catalog = dir.path().join("stores/catalog.json");
    let program = dir.path().join("Synthetic Tool.exe");
    fs::write(&program, b"test fixture, never executed").unwrap();
    let env = ScanEnv::for_fixtures(dir.path(), &NoRegistry);
    for operation in ["refresh", "settings", "add program"] {
        fs::write(&catalog, bytes).unwrap();
        let failed = match operation {
            "refresh" => library.refresh(&env).is_err(),
            "settings" => library.set_store_enabled(Store::Steam, false).is_err(),
            _ => library.add_program(&program, None).is_err(),
        };
        assert!(failed, "{operation} must reject an unreadable catalog");
        assert_eq!(
            fs::read(&catalog).unwrap(),
            bytes,
            "{operation} changed data"
        );
    }
}

#[test]
fn malformed_catalog_is_not_replaced_by_refresh_or_mutation() {
    damaged_catalog_is_preserved(br#"{"programs":[{"title":"Keep Me"}],"disabled":["steam"]"#);
}

#[test]
fn oversized_catalog_is_not_replaced_by_refresh_or_mutation() {
    damaged_catalog_is_preserved(&vec![b' '; 4 * 1024 * 1024 + 1]);
}

#[test]
fn invalid_utf8_catalog_is_not_replaced_by_refresh_or_mutation() {
    damaged_catalog_is_preserved(&[0xff, 0xfe, 0x00]);
}

#[test]
fn a_missing_catalog_initializes_and_keeps_programs_and_store_choices() {
    let dir = tempfile::tempdir().unwrap();
    let library = StoreLibrary::open(dir.path()).unwrap();
    let env = ScanEnv::for_fixtures(dir.path(), &NoRegistry);
    assert_eq!(library.refresh(&env).unwrap(), 0);
    library.set_store_enabled(Store::Steam, false).unwrap();
    let program = dir.path().join("Synthetic Tool.exe");
    fs::write(&program, b"test fixture, never executed").unwrap();
    let id = library.add_program(&program, None).unwrap();
    library.refresh(&env).unwrap();
    let reopened = StoreLibrary::open(dir.path()).unwrap();
    assert!(reopened.entries().iter().any(|entry| entry.id == id));
    assert!(
        !reopened
            .statuses(HostOs::Unix)
            .iter()
            .find(|s| s.key == "steam")
            .unwrap()
            .enabled
    );
}

#[cfg(target_os = "linux")]
#[test]
fn case_distinct_steam_libraries_both_contribute_games() {
    use gamestrike::stores::{scan, steam_libraries};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Steam");
    let upper = dir.path().join("Games");
    let lower = dir.path().join("games");
    fs::create_dir_all(root.join("steamapps")).unwrap();
    for (path, id) in [(&upper, "10"), (&lower, "20")] {
        fs::create_dir_all(path.join("steamapps/common/Test")).unwrap();
        fs::write(
            path.join(format!("steamapps/appmanifest_{id}.acf")),
            format!("\"AppState\" {{ \"appid\" \"{id}\" \"name\" \"Synthetic Test {id}\" \"StateFlags\" \"4\" \"installdir\" \"Test\" }}"),
        )
        .unwrap();
    }
    fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} \"2\" {{ \"path\" \"{}\" }} }}",
            upper.display(),
            lower.display()
        ),
    )
    .unwrap();
    assert_eq!(steam_libraries(&root).len(), 3);
    let env = ScanEnv {
        os: HostOs::Unix,
        registry: &NoRegistry,
        home: None,
        program_data: None,
        steam_root: Some(root),
        all_platforms: false,
    };
    let mut ids: Vec<_> = scan(Store::Steam, &env)
        .into_iter()
        .map(|game| game.store_id)
        .collect();
    ids.sort();
    assert_eq!(ids, ["10", "20"]);
}
