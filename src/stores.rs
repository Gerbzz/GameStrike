//! Installed games from other launchers (Steam, Epic Games, Ubisoft Connect, GOG Galaxy, the EA
//! app, Battle.net, and the Microsoft Store), plus programs the player adds by hand.
//!
//! Everything here is read-only and local: the scanners read the launchers' own install records
//! (Steam's `libraryfolders.vdf` and app manifests, Epic's `.item` manifests, registry entries, and
//! package manifests) and never write to them, sign in, read credentials, or use the network.
//! Registry access goes through the [`Registry`] trait, so every scanner is tested on every OS with
//! synthetic fixtures; the launcher supplies the real Windows registry.
//!
//! GameStrike keeps references to what it found (store, ID, title, install folder, how to launch)
//! in `LIBRARY/stores/catalog.json`, and the player's own data (favorite, hidden, title, covers,
//! plays) in `LIBRARY/stores/<game id>/`, which a rescan never deletes.

use crate::{HostOs, Result, cover_png_from_file, invalid, new_file, regular_file};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;

/// A launcher GameStrike can read installed games from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Store {
    Steam,
    Epic,
    Ea,
    Xbox,
    Gog,
    Ubisoft,
    Battlenet,
    Custom,
}

impl Store {
    /// In the order the sidebar lists them.
    pub const ALL: [Store; 8] = [
        Store::Steam,
        Store::Epic,
        Store::Ea,
        Store::Xbox,
        Store::Gog,
        Store::Ubisoft,
        Store::Battlenet,
        Store::Custom,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Store::Steam => "steam",
            Store::Epic => "epic",
            Store::Ea => "ea",
            Store::Xbox => "xbox",
            Store::Gog => "gog",
            Store::Ubisoft => "ubisoft",
            Store::Battlenet => "battlenet",
            Store::Custom => "custom",
        }
    }

    /// The store's name as the launcher shows it. No store logos are used anywhere.
    pub fn name(self) -> &'static str {
        match self {
            Store::Steam => "Steam",
            Store::Epic => "Epic Games",
            Store::Ea => "EA app",
            Store::Xbox => "Microsoft Store",
            Store::Gog => "GOG Galaxy",
            Store::Ubisoft => "Ubisoft Connect",
            Store::Battlenet => "Battle.net",
            Store::Custom => "Custom",
        }
    }

    pub fn from_key(key: &str) -> Option<Store> {
        Store::ALL.into_iter().find(|store| store.key() == key)
    }

    /// Where GameStrike knows how to find this store's games.
    pub fn supported_on(self, os: HostOs) -> bool {
        matches!(self, Store::Steam | Store::Custom) || os == HostOs::Windows
    }
}

/// How a game is started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Launch {
    /// A link the store's own launcher handles, such as `steam://rungameid/440`. Only the schemes
    /// in [`check_launch_uri`] are ever opened.
    Uri { uri: String },
    /// A program started directly, with its arguments passed as a list (never through a shell).
    Program {
        path: PathBuf,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<PathBuf>,
    },
}

/// One installed game a scanner found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub store: Store,
    pub store_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<PathBuf>,
    pub launch: Launch,
    /// Box art in the store launcher's own local cache, if it has any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_source: Option<PathBuf>,
}

// ---------- registry ----------

/// A registry root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
    LocalMachine,
    CurrentUser,
}

impl Hive {
    fn prefix(self) -> &'static str {
        match self {
            Hive::LocalMachine => "HKLM",
            Hive::CurrentUser => "HKCU",
        }
    }
}

/// Read-only registry access. Paths use backslashes and are case-insensitive. DWORD values are
/// returned as decimal strings.
pub trait Registry {
    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String>;
    fn string(&self, hive: Hive, path: &str, name: &str) -> Option<String>;
}

/// No registry, as on Linux and macOS.
pub struct NoRegistry;

impl Registry for NoRegistry {
    fn subkeys(&self, _hive: Hive, _path: &str) -> Vec<String> {
        Vec::new()
    }

    fn string(&self, _hive: Hive, _path: &str, _name: &str) -> Option<String> {
        None
    }
}

/// A registry read from a JSON snapshot: `{ "HKLM\\SOFTWARE\\...\\Key": { "Value": "data" } }`.
/// Used by the tests and by the launcher's synthetic capture fixtures.
#[derive(Default)]
pub struct SnapshotRegistry {
    /// Lowercase key path to (original key path, values by lowercase name).
    keys: BTreeMap<String, (String, BTreeMap<String, String>)>,
}

impl SnapshotRegistry {
    pub fn from_json(text: &str) -> Result<Self> {
        let raw: BTreeMap<String, BTreeMap<String, serde_json::Value>> = serde_json::from_str(text)
            .map_err(|_| invalid("the registry snapshot is not valid JSON"))?;
        let mut keys = BTreeMap::new();
        for (path, values) in raw {
            let path = path.trim_matches('\\').to_owned();
            let values = values
                .into_iter()
                .filter_map(|(name, value)| {
                    let text = match value {
                        serde_json::Value::String(text) => text,
                        serde_json::Value::Number(number) => number.to_string(),
                        _ => return None,
                    };
                    Some((name.to_lowercase(), text))
                })
                .collect();
            keys.insert(path.to_lowercase(), (path, values));
        }
        Ok(Self { keys })
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::from_json(&read_bounded(path, MAX_CATALOG_BYTES)?)
    }

    fn full(hive: Hive, path: &str) -> String {
        format!("{}\\{}", hive.prefix(), path.trim_matches('\\')).to_lowercase()
    }
}

impl Registry for SnapshotRegistry {
    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
        let full = Self::full(hive, path);
        let parent: Vec<&str> = full.split('\\').collect();
        let mut names = BTreeMap::new();
        for (original, _) in self.keys.values() {
            let segments: Vec<&str> = original.split('\\').collect();
            let under = segments.len() > parent.len()
                && segments
                    .iter()
                    .zip(&parent)
                    .all(|(segment, wanted)| segment.to_lowercase() == *wanted);
            if under {
                let name = segments[parent.len()];
                names
                    .entry(name.to_lowercase())
                    .or_insert_with(|| name.to_owned());
            }
        }
        names.into_values().collect()
    }

    fn string(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
        self.keys
            .get(&Self::full(hive, path))
            .and_then(|(_, values)| values.get(&name.to_lowercase()))
            .cloned()
    }
}

// ---------- where to look ----------

/// Where the scanners look. `for_this_computer` uses the real locations; fixtures point every
/// location into one folder.
pub struct ScanEnv<'a> {
    pub os: HostOs,
    pub registry: &'a dyn Registry,
    /// The user's home folder (Linux and macOS Steam locations).
    pub home: Option<PathBuf>,
    /// `C:\ProgramData` on Windows (Epic manifests, EA and Battle.net data).
    pub program_data: Option<PathBuf>,
    /// Overrides Steam discovery.
    pub steam_root: Option<PathBuf>,
    /// Scan every store whatever the OS (synthetic fixtures only).
    pub all_platforms: bool,
}

impl<'a> ScanEnv<'a> {
    pub fn for_this_computer(registry: &'a dyn Registry) -> Self {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        let program_data = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        Self {
            os: HostOs::current(),
            registry,
            home,
            program_data,
            steam_root: None,
            all_platforms: false,
        }
    }

    /// Synthetic fixtures: `DIR/steam` is a Steam folder, `DIR/ProgramData` stands in for
    /// `C:\ProgramData`, and the registry comes from the caller (usually `DIR/registry.json`).
    pub fn for_fixtures(dir: &Path, registry: &'a dyn Registry) -> Self {
        Self {
            os: HostOs::current(),
            registry,
            home: None,
            program_data: Some(dir.join("ProgramData")),
            steam_root: Some(dir.join("steam")),
            all_platforms: true,
        }
    }

    fn scans(&self, store: Store) -> bool {
        self.all_platforms || store.supported_on(self.os)
    }
}

/// Runs one store's scanner. Custom programs come from the catalog, not a scanner.
pub fn scan(store: Store, env: &ScanEnv) -> Vec<Found> {
    if !env.scans(store) {
        return Vec::new();
    }
    let mut found = match store {
        Store::Steam => scan_steam(env),
        Store::Epic => scan_epic(env),
        Store::Ea => scan_ea(env),
        Store::Xbox => scan_xbox(env),
        Store::Gog => scan_gog(env),
        Store::Ubisoft => scan_ubisoft(env),
        Store::Battlenet => scan_battlenet(env),
        Store::Custom => Vec::new(),
    };
    found.retain(|game| check_launch(&game.launch).is_ok());
    found.sort_by(|a, b| a.title.cmp(&b.title).then(a.store_id.cmp(&b.store_id)));
    found.dedup_by(|a, b| a.store_id == b.store_id);
    found
}

// ---------- Valve KeyValues (VDF) ----------

/// A parsed Valve KeyValues document: text values and nested maps, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Vdf {
    Text(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    /// The first child with this key, ignoring case.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(entries) => entries
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value),
            Vdf::Text(_) => None,
        }
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Text(text) => Some(text),
            Vdf::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Map(entries) => entries,
            Vdf::Text(_) => &[],
        }
    }
}

/// Parses Valve's text KeyValues format (`"key" "value"` and `"key" { ... }`), with escapes and
/// `//` comments. Depth and size are limited.
pub fn parse_vdf(input: &str) -> Result<Vdf> {
    if input.len() > MAX_CATALOG_BYTES as usize {
        return Err(invalid("the KeyValues file is too large").into());
    }
    let mut tokens = VdfTokens {
        chars: input.chars().peekable(),
    };
    let root = parse_vdf_map(&mut tokens, 0)?;
    Ok(Vdf::Map(root))
}

enum VdfToken {
    Text(String),
    Open,
    Close,
}

struct VdfTokens<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl VdfTokens<'_> {
    fn next_token(&mut self) -> Result<Option<VdfToken>> {
        loop {
            match self.chars.peek().copied() {
                None => return Ok(None),
                Some(c) if c.is_whitespace() => {
                    self.chars.next();
                }
                Some('/') => {
                    self.chars.next();
                    if self.chars.peek() == Some(&'/') {
                        for c in self.chars.by_ref() {
                            if c == '\n' {
                                break;
                            }
                        }
                    } else {
                        return Ok(Some(VdfToken::Text(self.bare("/"))));
                    }
                }
                Some('{') => {
                    self.chars.next();
                    return Ok(Some(VdfToken::Open));
                }
                Some('}') => {
                    self.chars.next();
                    return Ok(Some(VdfToken::Close));
                }
                Some('"') => {
                    self.chars.next();
                    let mut text = String::new();
                    loop {
                        match self.chars.next() {
                            None => return Err(invalid("unterminated string in KeyValues").into()),
                            Some('"') => break,
                            Some('\\') => match self.chars.next() {
                                Some('n') => text.push('\n'),
                                Some('t') => text.push('\t'),
                                Some(other) => text.push(other),
                                None => {
                                    return Err(invalid("unterminated string in KeyValues").into());
                                }
                            },
                            Some(c) => text.push(c),
                        }
                    }
                    return Ok(Some(VdfToken::Text(text)));
                }
                Some(_) => return Ok(Some(VdfToken::Text(self.bare("")))),
            }
        }
    }

    /// An unquoted token, up to whitespace, a brace, or a quote.
    fn bare(&mut self, start: &str) -> String {
        let mut text = start.to_owned();
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                break;
            }
            text.push(c);
            self.chars.next();
        }
        text
    }
}

fn parse_vdf_map(tokens: &mut VdfTokens, depth: usize) -> Result<Vec<(String, Vdf)>> {
    if depth > 32 {
        return Err(invalid("KeyValues nesting is too deep").into());
    }
    let mut entries = Vec::new();
    loop {
        let key = match tokens.next_token()? {
            None if depth == 0 => return Ok(entries),
            None => return Err(invalid("unexpected end of KeyValues").into()),
            Some(VdfToken::Close) if depth > 0 => return Ok(entries),
            Some(VdfToken::Text(key)) => key,
            Some(_) => return Err(invalid("unexpected brace in KeyValues").into()),
        };
        let value = match tokens.next_token()? {
            Some(VdfToken::Text(text)) => Vdf::Text(text),
            Some(VdfToken::Open) => Vdf::Map(parse_vdf_map(tokens, depth + 1)?),
            _ => return Err(invalid("a KeyValues key has no value").into()),
        };
        if entries.len() >= 100_000 {
            return Err(invalid("too many KeyValues entries").into());
        }
        entries.push((key, value));
    }
}

// ---------- Steam ----------

/// Steam's redistributables, runtimes, and compatibility tools: not games.
const STEAM_TOOLS: [&str; 10] = [
    "228980", "1070560", "1391110", "1628350", "1493710", "2180100", "1826330", "1161040",
    "2348590", "2805730",
];

fn steam_roots(env: &ScanEnv) -> Vec<PathBuf> {
    if let Some(root) = &env.steam_root {
        return vec![root.clone()];
    }
    let mut roots = Vec::new();
    match env.os {
        HostOs::Windows => {
            for (hive, path, name) in [
                (Hive::CurrentUser, "Software\\Valve\\Steam", "SteamPath"),
                (
                    Hive::LocalMachine,
                    "SOFTWARE\\WOW6432Node\\Valve\\Steam",
                    "InstallPath",
                ),
                (Hive::LocalMachine, "SOFTWARE\\Valve\\Steam", "InstallPath"),
            ] {
                if let Some(value) = env.registry.string(hive, path, name) {
                    roots.push(PathBuf::from(value));
                }
            }
        }
        HostOs::Unix => {
            if let Some(home) = &env.home {
                roots.push(home.join(".steam").join("steam"));
                roots.push(home.join(".local").join("share").join("Steam"));
                roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
            }
        }
        HostOs::MacOs => {
            if let Some(home) = &env.home {
                roots.push(home.join("Library/Application Support/Steam"));
            }
        }
    }
    let mut seen = BTreeSet::new();
    roots
        .into_iter()
        .filter(|root| root.join("steamapps").is_dir())
        .filter(|root| seen.insert(fs::canonicalize(root).unwrap_or_else(|_| root.clone())))
        .collect()
}

/// The library folders listed in `steamapps/libraryfolders.vdf` (both the current format with
/// `"path"` entries and the older one with bare paths), plus the Steam folder itself.
pub fn steam_libraries(root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![root.to_path_buf()];
    let file = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(document) = read_bounded(&file, MAX_CATALOG_BYTES).and_then(|text| parse_vdf(&text)) {
        let folders = document
            .get("libraryfolders")
            .or_else(|| document.get("LibraryFolders"));
        for (key, value) in folders.map(Vdf::entries).unwrap_or_default() {
            let path = match value {
                Vdf::Map(_) => value.text("path").map(str::to_owned),
                Vdf::Text(text) if key.chars().all(|c| c.is_ascii_digit()) => Some(text.clone()),
                Vdf::Text(_) => None,
            };
            if let Some(path) = path.filter(|path| !path.is_empty()) {
                libraries.push(PathBuf::from(path));
            }
        }
    }
    let mut seen = BTreeSet::new();
    libraries.retain(|library| {
        let key = fs::canonicalize(library)
            .unwrap_or_else(|_| library.clone())
            .to_string_lossy()
            .to_lowercase();
        seen.insert(key)
    });
    libraries
}

/// One `appmanifest_<appid>.acf`: the game, or `None` for tools, redistributables, and games that
/// are not fully installed (`StateFlags` without the installed bit, 4).
pub fn steam_app(library: &Path, manifest: &str) -> Option<(String, String, PathBuf)> {
    let document = parse_vdf(manifest).ok()?;
    let app = document.get("AppState")?;
    let appid = app.text("appid")?.trim().to_owned();
    if appid.is_empty() || !appid.chars().all(|c| c.is_ascii_digit()) || appid.len() > 12 {
        return None;
    }
    let flags: u32 = app.text("StateFlags")?.trim().parse().ok()?;
    if (flags & 4) == 0 || STEAM_TOOLS.contains(&appid.as_str()) {
        return None;
    }
    let name = clean_store_title(app.text("name")?)?;
    let lower = name.to_lowercase();
    if lower.starts_with("proton ")
        || lower.starts_with("steam linux runtime")
        || lower.starts_with("steamworks common")
    {
        return None;
    }
    let install = app.text("installdir").unwrap_or_default();
    let path = library.join("steamapps").join("common").join(install);
    Some((appid, name, path))
}

/// Steam's own cached library art for an app: `appcache/librarycache/<appid>_library_600x900.jpg`,
/// or the newer per-app folders (`<appid>/library_600x900.jpg`, possibly one level deeper).
pub fn steam_cover(root: &Path, appid: &str) -> Option<PathBuf> {
    let cache = root.join("appcache").join("librarycache");
    let flat = cache.join(format!("{appid}_library_600x900.jpg"));
    if regular_file(&flat) {
        return Some(flat);
    }
    let folder = cache.join(appid);
    let direct = folder.join("library_600x900.jpg");
    if regular_file(&direct) {
        return Some(direct);
    }
    let mut nested: Vec<PathBuf> = fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|item| item.path().join("library_600x900.jpg"))
        .filter(|path| regular_file(path))
        .collect();
    nested.sort();
    nested.into_iter().next()
}

fn scan_steam(env: &ScanEnv) -> Vec<Found> {
    let mut found = Vec::new();
    for root in steam_roots(env) {
        for library in steam_libraries(&root) {
            let Ok(items) = fs::read_dir(library.join("steamapps")) else {
                continue;
            };
            for item in items.flatten().take(20_000) {
                let name = item.file_name().to_string_lossy().into_owned();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let Ok(text) = read_bounded(&item.path(), 1024 * 1024) else {
                    continue;
                };
                if let Some((appid, title, path)) = steam_app(&library, &text) {
                    found.push(Found {
                        store: Store::Steam,
                        cover_source: steam_cover(&root, &appid),
                        launch: Launch::Uri {
                            uri: format!("steam://rungameid/{appid}"),
                        },
                        store_id: appid,
                        title,
                        install_path: Some(path),
                    });
                }
            }
        }
    }
    found
}

// ---------- Epic Games ----------

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EpicItem {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    install_location: Option<String>,
    #[serde(default)]
    catalog_namespace: Option<String>,
    #[serde(default)]
    catalog_item_id: Option<String>,
    #[serde(default)]
    main_game_app_name: Option<String>,
    #[serde(default)]
    app_categories: Option<Vec<String>>,
    #[serde(default, rename = "bIsIncompleteInstall")]
    incomplete: Option<bool>,
}

/// One Epic `.item` manifest, or `None` for add-ons, engines, and incomplete installs. The launch
/// link has the form Epic's launcher registers, `namespace:catalogItemId:appName` with the colons
/// encoded, or just the app name when the catalog IDs are missing.
pub fn epic_game(manifest: &str) -> Option<Found> {
    let item: EpicItem = serde_json::from_str(manifest).ok()?;
    let app = item.app_name.filter(|id| store_id_ok(id))?;
    if item.incomplete == Some(true) {
        return None;
    }
    if item
        .main_game_app_name
        .as_deref()
        .is_some_and(|main| !main.is_empty() && main != app)
    {
        return None;
    }
    if let Some(categories) = &item.app_categories
        && !categories.iter().any(|category| category == "games")
    {
        return None;
    }
    let location = item.install_location.filter(|path| !path.is_empty())?;
    let title = item
        .display_name
        .as_deref()
        .and_then(clean_store_title)
        .unwrap_or_else(|| app.clone());
    let uri = match (item.catalog_namespace, item.catalog_item_id) {
        (Some(namespace), Some(catalog)) if store_id_ok(&namespace) && store_id_ok(&catalog) => {
            format!(
                "com.epicgames.launcher://apps/{namespace}%3A{catalog}%3A{app}?action=launch&silent=true"
            )
        }
        _ => format!("com.epicgames.launcher://apps/{app}?action=launch&silent=true"),
    };
    Some(Found {
        store: Store::Epic,
        store_id: app,
        title,
        install_path: Some(PathBuf::from(location)),
        launch: Launch::Uri { uri },
        cover_source: None,
    })
}

fn scan_epic(env: &ScanEnv) -> Vec<Found> {
    let Some(data) = &env.program_data else {
        return Vec::new();
    };
    let folder = data.join("Epic/EpicGamesLauncher/Data/Manifests");
    let Ok(items) = fs::read_dir(folder) else {
        return Vec::new();
    };
    items
        .flatten()
        .take(5000)
        .filter(|item| item.path().extension().is_some_and(|ext| ext == "item"))
        .filter_map(|item| read_bounded(&item.path(), 1024 * 1024).ok())
        .filter_map(|text| epic_game(&text))
        .filter(|game| game.install_path.as_deref().is_some_and(Path::is_dir))
        .collect()
}

// ---------- GOG Galaxy ----------

const GOG_GAMES: &str = "SOFTWARE\\WOW6432Node\\GOG.com\\Games";

fn scan_gog(env: &ScanEnv) -> Vec<Found> {
    let registry = env.registry;
    let galaxy = registry
        .string(
            Hive::LocalMachine,
            "SOFTWARE\\WOW6432Node\\GOG.com\\GalaxyClient\\paths",
            "client",
        )
        .map(|folder| {
            let exe = registry
                .string(
                    Hive::LocalMachine,
                    "SOFTWARE\\WOW6432Node\\GOG.com\\GalaxyClient",
                    "clientExecutable",
                )
                .unwrap_or_else(|| "GalaxyClient.exe".to_owned());
            PathBuf::from(folder).join(exe)
        })
        .filter(|path| regular_file(path));
    let mut found = Vec::new();
    for id in registry.subkeys(Hive::LocalMachine, GOG_GAMES) {
        let key = format!("{GOG_GAMES}\\{id}");
        let value = |name: &str| registry.string(Hive::LocalMachine, &key, name);
        if !id.chars().all(|c| c.is_ascii_digit()) || id.len() > 20 {
            continue;
        }
        // Add-ons name the game they belong to.
        if value("dependsOn").is_some_and(|parent| !parent.trim().is_empty()) {
            continue;
        }
        let Some(path) = value("path")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
        else {
            continue;
        };
        let Some(title) = value("gameName").as_deref().and_then(clean_store_title) else {
            continue;
        };
        let launch = match &galaxy {
            Some(client) => Launch::Program {
                path: client.clone(),
                args: vec![
                    "/command=runGame".to_owned(),
                    format!("/gameId={id}"),
                    format!("/path={}", path.display()),
                ],
                working_dir: client.parent().map(Path::to_path_buf),
            },
            None => {
                let Some(exe) = value("exe")
                    .map(PathBuf::from)
                    .filter(|exe| regular_file(exe))
                else {
                    continue;
                };
                Launch::Program {
                    path: exe,
                    args: value("launchParam")
                        .map(|param| split_arguments(&param))
                        .unwrap_or_default(),
                    working_dir: value("workingDir")
                        .map(PathBuf::from)
                        .filter(|dir| dir.is_dir())
                        .or_else(|| Some(path.clone())),
                }
            }
        };
        found.push(Found {
            store: Store::Gog,
            store_id: id,
            title,
            install_path: Some(path),
            launch,
            cover_source: None,
        });
    }
    found
}

// ---------- Ubisoft Connect ----------

const UBISOFT_INSTALLS: &str = "SOFTWARE\\WOW6432Node\\Ubisoft\\Launcher\\Installs";
const UNINSTALL_KEYS: [&str; 2] = [
    "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
];

fn scan_ubisoft(env: &ScanEnv) -> Vec<Found> {
    let registry = env.registry;
    let mut found = Vec::new();
    for id in registry.subkeys(Hive::LocalMachine, UBISOFT_INSTALLS) {
        if !id.chars().all(|c| c.is_ascii_digit()) || id.len() > 12 {
            continue;
        }
        let key = format!("{UBISOFT_INSTALLS}\\{id}");
        let Some(path) = registry
            .string(Hive::LocalMachine, &key, "InstallDir")
            .map(|dir| PathBuf::from(dir.trim_end_matches(['/', '\\'])))
            .filter(|path| path.is_dir())
        else {
            continue;
        };
        // Ubisoft Connect registers each game for uninstall as "Uplay Install <id>".
        let title = UNINSTALL_KEYS
            .iter()
            .find_map(|root| {
                registry.string(
                    Hive::LocalMachine,
                    &format!("{root}\\Uplay Install {id}"),
                    "DisplayName",
                )
            })
            .as_deref()
            .and_then(clean_store_title)
            .or_else(|| folder_title(&path));
        let Some(title) = title else {
            continue;
        };
        found.push(Found {
            store: Store::Ubisoft,
            launch: Launch::Uri {
                uri: format!("uplay://launch/{id}/0"),
            },
            store_id: id,
            title,
            install_path: Some(path),
            cover_source: None,
        });
    }
    found
}

// ---------- EA app ----------

const EA_KEYS: [&str; 3] = [
    "SOFTWARE\\WOW6432Node\\EA Games",
    "SOFTWARE\\WOW6432Node\\Electronic Arts",
    "SOFTWARE\\EA Games",
];

/// The content (offer) IDs in a game's `__Installer/installerdata.xml`.
pub fn ea_content_ids(installer_data: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = installer_data;
    while let Some(start) = rest.find("<contentID>") {
        rest = &rest[start + "<contentID>".len()..];
        let Some(end) = rest.find("</contentID>") else {
            break;
        };
        let id = rest[..end].trim();
        // EA content IDs can carry a colon, as in "OFB-EAST:109552154".
        let valid = !id.is_empty()
            && id.len() <= 64
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c));
        if valid && !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
        }
        rest = &rest[end..];
        if ids.len() >= 16 {
            break;
        }
    }
    ids
}

/// Best effort. The EA app keeps its own install list encrypted, so GameStrike reads the registry
/// entries EA games write, plus each game's `installerdata.xml`, and launches through the
/// `origin2://` link the EA app handles. Games without a content ID are skipped.
fn scan_ea(env: &ScanEnv) -> Vec<Found> {
    let registry = env.registry;
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    for root in EA_KEYS {
        for name in registry.subkeys(Hive::LocalMachine, root) {
            let lower = name.to_lowercase();
            if lower.contains("ea desktop") || lower.contains("ea core") || lower == "origin" {
                continue;
            }
            let key = format!("{root}\\{name}");
            let dir = ["Install Dir", "InstallDir", "GameInstallDir"]
                .iter()
                .find_map(|value| registry.string(Hive::LocalMachine, &key, value));
            let title = registry
                .string(Hive::LocalMachine, &key, "DisplayName")
                .unwrap_or_else(|| name.clone());
            if let Some(dir) = dir {
                candidates.push((title, PathBuf::from(dir.trim_end_matches('\\'))));
            }
        }
    }
    for root in UNINSTALL_KEYS {
        for name in registry.subkeys(Hive::LocalMachine, root) {
            let key = format!("{root}\\{name}");
            let publisher = registry
                .string(Hive::LocalMachine, &key, "Publisher")
                .unwrap_or_default();
            if !publisher.eq_ignore_ascii_case("Electronic Arts") {
                continue;
            }
            let title = registry.string(Hive::LocalMachine, &key, "DisplayName");
            let dir = registry.string(Hive::LocalMachine, &key, "InstallLocation");
            if let (Some(title), Some(dir)) = (title, dir) {
                candidates.push((title, PathBuf::from(dir.trim_end_matches('\\'))));
            }
        }
    }
    let mut found: Vec<Found> = Vec::new();
    for (title, path) in candidates {
        if !path.is_dir() {
            continue;
        }
        let data = path.join("__Installer").join("installerdata.xml");
        let ids = read_bounded(&data, 1024 * 1024)
            .map(|text| ea_content_ids(&text))
            .unwrap_or_default();
        let (Some(first), Some(title)) = (ids.first().cloned(), clean_store_title(&title)) else {
            continue;
        };
        if found.iter().any(|game| game.store_id == first) {
            continue;
        }
        found.push(Found {
            store: Store::Ea,
            launch: Launch::Uri {
                uri: format!(
                    "origin2://game/launch?offerIds={}&autoDownload=1",
                    ids.join(",")
                ),
            },
            store_id: first,
            title,
            install_path: Some(path),
            cover_source: None,
        });
    }
    found
}

// ---------- Battle.net ----------

/// Battle.net agent product IDs (from the uninstall entry's `--uid=`) and the product codes its
/// `--exec="launch <code>"` option takes. Games not in this table open Battle.net itself.
const BATTLENET_PRODUCTS: [(&str, &str); 21] = [
    ("wow", "WoW"),
    ("wow_classic", "WoWC"),
    ("wow_classic_era", "WoWCE"),
    ("diablo3", "D3"),
    ("fenris", "Fen"),
    ("osi", "OSI"),
    ("hero", "Hero"),
    ("prometheus", "Pro"),
    ("s1", "S1"),
    ("s2", "S2"),
    ("w3", "W3"),
    ("hs_beta", "WTCG"),
    ("rtro", "RTRO"),
    ("wlby", "WLBY"),
    ("viper", "VIPR"),
    ("odin", "ODIN"),
    ("lazarus", "LAZR"),
    ("zeus", "ZEUS"),
    ("fore", "FORE"),
    ("anbs", "ANBS"),
    ("gryphon", "GRY"),
];

/// The agent product ID in a Battle.net uninstall command line (`--uid=fenris`).
pub fn battlenet_uid(uninstall: &str) -> Option<String> {
    let start = uninstall.find("--uid=")? + "--uid=".len();
    let uid: String = uninstall[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    (!uid.is_empty() && uid.len() <= 40).then_some(uid)
}

fn scan_battlenet(env: &ScanEnv) -> Vec<Found> {
    let registry = env.registry;
    let mut client: Option<PathBuf> = None;
    let mut games: Vec<(String, String, PathBuf)> = Vec::new();
    for root in UNINSTALL_KEYS {
        for name in registry.subkeys(Hive::LocalMachine, root) {
            let key = format!("{root}\\{name}");
            let value = |field: &str| registry.string(Hive::LocalMachine, &key, field);
            let publisher = value("Publisher").unwrap_or_default();
            if !publisher.to_lowercase().contains("blizzard") {
                continue;
            }
            let title = value("DisplayName").unwrap_or_default();
            let location = value("InstallLocation").map(PathBuf::from);
            if title.eq_ignore_ascii_case("Battle.net") {
                client = location
                    .map(|dir| dir.join("Battle.net.exe"))
                    .filter(|exe| regular_file(exe));
                continue;
            }
            let uid = value("UninstallString").as_deref().and_then(battlenet_uid);
            if let (Some(uid), Some(location)) = (uid, location) {
                games.push((uid, title, location));
            }
        }
    }
    let mut found = Vec::new();
    for (uid, title, path) in games {
        if uid == "battle.net" || !path.is_dir() {
            continue;
        }
        let Some(title) = clean_store_title(&title) else {
            continue;
        };
        let code = BATTLENET_PRODUCTS
            .iter()
            .find(|(known, _)| *known == uid)
            .map(|(_, code)| *code);
        let launch = match (&client, code) {
            (Some(exe), Some(code)) => Launch::Program {
                path: exe.clone(),
                args: vec![format!("--exec=launch {code}")],
                working_dir: exe.parent().map(Path::to_path_buf),
            },
            (None, Some(code)) => Launch::Uri {
                uri: format!("battlenet://{code}"),
            },
            (Some(exe), None) => Launch::Program {
                path: exe.clone(),
                args: Vec::new(),
                working_dir: exe.parent().map(Path::to_path_buf),
            },
            (None, None) => continue,
        };
        found.push(Found {
            store: Store::Battlenet,
            store_id: uid,
            title,
            install_path: Some(path),
            launch,
            cover_source: None,
        });
    }
    found
}

// ---------- Microsoft Store and Xbox app ----------

const PACKAGES: &str = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages";

/// The value of `attribute="..."` in the first element named `element` in an XML text.
pub fn xml_attribute(text: &str, element: &str, attribute: &str) -> Option<String> {
    let open = format!("<{element}");
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        let tail = &rest[start + open.len()..];
        rest = tail;
        if !tail.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            continue;
        }
        let end = tail.find('>')?;
        let tag = &tail[..end];
        let needle = format!("{attribute}=\"");
        let mut search = tag;
        while let Some(at) = search.find(&needle) {
            let before = search[..at].chars().last();
            let value = &search[at + needle.len()..];
            if before.is_none_or(char::is_whitespace) {
                let close = value.find('"')?;
                return Some(xml_unescape(&value[..close]));
            }
            search = value;
        }
        return None;
    }
    None
}

fn xml_unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// A game package from its folder: only packages with a `MicrosoftGame.config` (games built with
/// Microsoft's game development kit) count. `full_name` is the package full name
/// (`Name_Version_Arch_ResourceId_PublisherId`); the launch target is `shell:AppsFolder\<AUMID>`.
pub fn xbox_game(full_name: &str, root: &Path) -> Option<Found> {
    let config = read_bounded(&root.join("MicrosoftGame.config"), 1024 * 1024).ok()?;
    let manifest = read_bounded(&root.join("AppxManifest.xml"), 2 * 1024 * 1024).ok()?;
    let parts: Vec<&str> = full_name.split('_').collect();
    let (name, publisher) = match parts.as_slice() {
        [name, _version, _arch, _resource, publisher] => (*name, *publisher),
        _ => return None,
    };
    let app = xml_attribute(&manifest, "Application", "Id")?;
    if ![name, publisher, app.as_str()]
        .iter()
        .all(|part| store_id_ok(part))
    {
        return None;
    }
    let title = xml_attribute(&config, "ShellVisuals", "DefaultDisplayName")
        .filter(|title| !title.starts_with("ms-resource:"))
        .as_deref()
        .and_then(clean_store_title)
        .or_else(|| clean_store_title(name))?;
    let cover_source = ["Square480x480Logo", "Square150x150Logo", "StoreLogo"]
        .iter()
        .filter_map(|attribute| xml_attribute(&config, "ShellVisuals", attribute))
        .map(|relative| root.join(relative.replace('\\', "/")))
        .find(|path| regular_file(path));
    let family = format!("{name}_{publisher}");
    Some(Found {
        store: Store::Xbox,
        store_id: family.clone(),
        title,
        install_path: Some(root.to_path_buf()),
        launch: Launch::Uri {
            uri: format!("shell:AppsFolder\\{family}!{app}"),
        },
        cover_source,
    })
}

fn scan_xbox(env: &ScanEnv) -> Vec<Found> {
    let registry = env.registry;
    let mut found = Vec::new();
    for full_name in registry
        .subkeys(Hive::CurrentUser, PACKAGES)
        .into_iter()
        .take(5000)
    {
        let key = format!("{PACKAGES}\\{full_name}");
        let Some(root) = registry
            .string(Hive::CurrentUser, &key, "PackageRootFolder")
            .map(PathBuf::from)
        else {
            continue;
        };
        if let Some(game) = xbox_game(&full_name, &root) {
            found.push(game);
        }
    }
    found
}

// ---------- launch safety ----------

/// The only links GameStrike opens, each a store launcher's own scheme.
const LAUNCH_SCHEMES: [&str; 6] = [
    "steam://rungameid/",
    "com.epicgames.launcher://apps/",
    "uplay://launch/",
    "origin2://game/launch?",
    "battlenet://",
    "shell:AppsFolder\\",
];

/// Accepts a launch link only if it uses one of the store schemes above and contains nothing but
/// URL-safe characters, so no link can smuggle quotes, spaces, or another command.
pub fn check_launch_uri(uri: &str) -> Result<()> {
    if uri.len() > 512 || !LAUNCH_SCHEMES.iter().any(|scheme| uri.starts_with(scheme)) {
        return Err(invalid("GameStrike only opens store launcher links").into());
    }
    let safe = |c: char| c.is_ascii_alphanumeric() || "-._~:/?&=%!\\,".contains(c);
    if !uri.chars().all(safe) {
        return Err(invalid("the store link contains unexpected characters").into());
    }
    Ok(())
}

/// Checks a launch target: a link per [`check_launch_uri`], or an absolute program path with
/// bounded, NUL-free arguments.
pub fn check_launch(launch: &Launch) -> Result<()> {
    match launch {
        Launch::Uri { uri } => check_launch_uri(uri),
        Launch::Program {
            path,
            args,
            working_dir,
        } => {
            if !path.is_absolute() || working_dir.as_deref().is_some_and(|dir| !dir.is_absolute()) {
                return Err(invalid("program paths must be absolute").into());
            }
            check_arguments(args)
        }
    }
}

const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_CHARS: usize = 1024;

pub fn check_arguments(args: &[String]) -> Result<()> {
    if args.len() > MAX_ARGUMENTS
        || args.iter().map(|arg| arg.chars().count()).sum::<usize>() > MAX_ARGUMENT_CHARS
        || args.iter().any(|arg| arg.contains('\0'))
    {
        return Err(invalid("use up to 32 arguments and 1024 characters in total").into());
    }
    Ok(())
}

/// Splits a command line into arguments: whitespace separates them, double quotes group them.
/// Used for arguments the player types and for GOG's stored launch parameters; the result is
/// passed to the program as a list, never through a shell.
pub fn split_arguments(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    args.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            c => {
                current.push(c);
                any = true;
            }
        }
    }
    if any {
        args.push(current);
    }
    args
}

// ---------- helpers ----------

/// Store IDs that may appear in a launch link: ASCII letters, digits, and `-_.`.
fn store_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Trims, drops control characters and trademark signs, and limits titles to 80 characters.
fn clean_store_title(raw: &str) -> Option<String> {
    let title: String = raw
        .chars()
        .filter(|&c| !c.is_control() && !matches!(c, '™' | '®' | '©'))
        .collect();
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    Some(title.chars().take(80).collect())
}

fn folder_title(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(clean_store_title)
}

const MAX_CATALOG_BYTES: u64 = 4 * 1024 * 1024;

fn read_bounded(path: &Path, limit: u64) -> Result<String> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("file is missing or too large").into());
    }
    let mut text = String::new();
    File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut text)?;
    Ok(text)
}

/// GameStrike's ID for a store game: the same for the same store and store ID on every scan, and
/// never equal to an HTML game's random ID in practice.
pub fn store_game_id(store: Store, store_id: &str) -> Uuid {
    if store == Store::Custom
        && let Ok(id) = Uuid::parse_str(store_id)
    {
        return id;
    }
    let digest = Sha256::digest(format!("gamestrike-store:{}:{store_id}", store.key()).as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

// ---------- the catalog ----------

/// A program the player added by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomProgram {
    pub id: Uuid,
    pub title: String,
    pub path: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub added_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CatalogEntry {
    id: Uuid,
    found: Found,
    installed: bool,
    first_seen: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Catalog {
    #[serde(default)]
    games: Vec<CatalogEntry>,
    #[serde(default)]
    programs: Vec<CustomProgram>,
    #[serde(default)]
    disabled: BTreeSet<Store>,
    #[serde(default)]
    scanned_at: Option<u64>,
}

/// The player's own data for one store game. A rescan never touches it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoreMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_played: Option<u64>,
    #[serde(default)]
    play_count: u32,
}

/// One store game as the launcher shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StoreEntry {
    pub id: Uuid,
    pub store: Store,
    pub store_name: &'static str,
    pub store_id: String,
    pub title: String,
    /// The title the store reported, before any rename.
    pub store_title: String,
    pub installed: bool,
    pub install_path: Option<PathBuf>,
    pub favorite: bool,
    pub hidden: bool,
    pub last_played: Option<u64>,
    pub play_count: u32,
    pub added_at: Option<u64>,
    /// "custom" (the player's image) or "store" (the store launcher's cached art), or none.
    pub cover: Option<&'static str>,
    pub cover_version: u64,
    pub cover_undo: bool,
    /// How it launches: "uri" through the store's launcher, or "program" directly.
    pub launch_kind: &'static str,
    /// For custom programs: their arguments and working folder.
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
}

/// Per store: whether it is scanned, whether this OS is supported, and how many games it has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StoreStatus {
    pub key: &'static str,
    pub name: &'static str,
    pub enabled: bool,
    pub supported: bool,
    pub games: usize,
}

/// Store games in `LIBRARY/stores`.
pub struct StoreLibrary {
    root: PathBuf,
    lock: Mutex<()>,
}

const STORE_COVER: &str = "store-cover.png";
const STORE_COVER_KEY: &str = "store-cover.key";

impl StoreLibrary {
    pub fn open(library_root: &Path) -> Result<Self> {
        let root = library_root.join("stores");
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            lock: Mutex::new(()),
        })
    }

    fn catalog(&self) -> Catalog {
        read_bounded(&self.root.join("catalog.json"), MAX_CATALOG_BYTES)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save_catalog(&self, catalog: &Catalog) -> Result<()> {
        write_json(&self.root.join("catalog.json"), catalog, MAX_CATALOG_BYTES)
    }

    /// Changes the catalog under the lock.
    fn update<T>(&self, change: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut catalog = self.catalog();
        let result = change(&mut catalog)?;
        self.save_catalog(&catalog)?;
        Ok(result)
    }

    fn game_dir(&self, id: Uuid) -> PathBuf {
        self.root.join(id.to_string())
    }

    fn meta(&self, id: Uuid) -> StoreMeta {
        read_bounded(&self.game_dir(id).join("meta.json"), 8192)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn change_meta(&self, id: Uuid, change: impl FnOnce(&mut StoreMeta)) -> Result<()> {
        if !self.contains(id) {
            return Err(invalid("game ID was not found in the library").into());
        }
        let folder = self.game_dir(id);
        fs::create_dir_all(&folder)?;
        let mut meta = self.meta(id);
        change(&mut meta);
        write_json(&folder.join("meta.json"), &meta, 8192)
    }

    /// Whether the player has made this game their own (so it stays listed after an uninstall).
    fn has_player_data(&self, id: Uuid) -> bool {
        let meta = self.meta(id);
        meta != StoreMeta::default() || regular_file(&self.game_dir(id).join(CUSTOM_COVER_NAME))
    }

    pub fn contains(&self, id: Uuid) -> bool {
        let catalog = self.catalog();
        catalog.games.iter().any(|entry| entry.id == id)
            || catalog.programs.iter().any(|program| program.id == id)
    }

    /// Scans every enabled store and merges the results: new games are added, games still found
    /// are updated, and games no longer found are marked as not installed (and dropped if the
    /// player never favorited, renamed, played, hid, or gave them a cover). Store covers are
    /// decoded with the same limits as any cover and cached as PNG. Returns the installed count.
    pub fn refresh(&self, env: &ScanEnv) -> Result<usize> {
        let disabled = self.catalog().disabled;
        let mut scanned: Vec<(Store, Vec<Found>)> = Vec::new();
        for store in Store::ALL {
            if store != Store::Custom && !disabled.contains(&store) {
                scanned.push((store, scan(store, env)));
            }
        }
        let now = crate::now_unix();
        let found: Vec<Found> = scanned
            .iter()
            .flat_map(|(_, games)| games.clone())
            .collect();
        for game in &found {
            if let Some(source) = &game.cover_source {
                let _ = self.cache_store_cover(store_game_id(game.store, &game.store_id), source);
            }
        }
        let keep: BTreeSet<Uuid> = self
            .catalog()
            .games
            .iter()
            .filter(|entry| self.has_player_data(entry.id))
            .map(|entry| entry.id)
            .collect();
        self.update(|catalog| {
            let scanned_stores: BTreeSet<Store> = scanned.iter().map(|(store, _)| *store).collect();
            for entry in &mut catalog.games {
                if scanned_stores.contains(&entry.found.store) {
                    entry.installed = false;
                }
            }
            for game in found {
                let id = store_game_id(game.store, &game.store_id);
                match catalog.games.iter_mut().find(|entry| entry.id == id) {
                    Some(entry) => {
                        entry.found = game;
                        entry.installed = true;
                    }
                    None => catalog.games.push(CatalogEntry {
                        id,
                        found: game,
                        installed: true,
                        first_seen: now,
                    }),
                }
            }
            catalog
                .games
                .retain(|entry| entry.installed || keep.contains(&entry.id));
            catalog.scanned_at = Some(now);
            Ok(catalog.games.iter().filter(|entry| entry.installed).count())
        })
    }

    fn cache_store_cover(&self, id: Uuid, source: &Path) -> Result<()> {
        let metadata = fs::metadata(source)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |time| time.as_millis());
        let key = format!("{}|{}|{modified}", source.display(), metadata.len());
        let folder = self.game_dir(id);
        if read_bounded(&folder.join(STORE_COVER_KEY), 8192).is_ok_and(|known| known == key)
            && regular_file(&folder.join(STORE_COVER))
        {
            return Ok(());
        }
        let png = cover_png_from_file(source)?;
        fs::create_dir_all(&folder)?;
        write_file_atomic(&folder.join(STORE_COVER), &png)?;
        write_file_atomic(&folder.join(STORE_COVER_KEY), key.as_bytes())
    }

    /// Store games for the launcher, without stores the player turned off. Games that are no longer
    /// installed are listed only if the player has made them their own.
    pub fn entries(&self) -> Vec<StoreEntry> {
        let catalog = self.catalog();
        let mut entries: Vec<StoreEntry> = catalog
            .games
            .iter()
            .filter(|entry| !catalog.disabled.contains(&entry.found.store))
            .map(|entry| self.entry(entry.id, &entry.found, entry.installed, entry.first_seen))
            .collect();
        if !catalog.disabled.contains(&Store::Custom) {
            for program in &catalog.programs {
                let found = program_found(program);
                let installed = regular_file(&program.path) || program.path.is_dir();
                entries.push(self.entry(program.id, &found, installed, program.added_at));
            }
        }
        entries.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        entries
    }

    fn entry(&self, id: Uuid, found: &Found, installed: bool, first_seen: u64) -> StoreEntry {
        let meta = self.meta(id);
        let folder = self.game_dir(id);
        let (launch_kind, args, working_dir) = match &found.launch {
            Launch::Uri { .. } => ("uri", Vec::new(), None),
            Launch::Program {
                args, working_dir, ..
            } if found.store == Store::Custom => ("program", args.clone(), working_dir.clone()),
            Launch::Program { .. } => ("program", Vec::new(), None),
        };
        StoreEntry {
            id,
            store: found.store,
            store_name: found.store.name(),
            store_id: found.store_id.clone(),
            title: meta.title.clone().unwrap_or_else(|| found.title.clone()),
            store_title: found.title.clone(),
            installed,
            install_path: found.install_path.clone(),
            favorite: meta.favorite,
            hidden: meta.hidden,
            last_played: meta.last_played,
            play_count: meta.play_count,
            added_at: (first_seen > 0).then_some(first_seen),
            cover: if regular_file(&folder.join(CUSTOM_COVER_NAME)) {
                Some("custom")
            } else if regular_file(&folder.join(STORE_COVER)) {
                Some("store")
            } else {
                None
            },
            cover_version: self.cover_file(id).map_or(0, |path| modified_millis(&path)),
            cover_undo: regular_file(&folder.join(PREVIOUS_COVER_NAME)),
            launch_kind,
            args,
            working_dir,
        }
    }

    /// Which stores exist on this OS, which are turned on, and how many games each lists.
    pub fn statuses(&self, os: HostOs) -> Vec<StoreStatus> {
        let catalog = self.catalog();
        let entries = self.entries();
        Store::ALL
            .into_iter()
            .map(|store| StoreStatus {
                key: store.key(),
                name: store.name(),
                enabled: !catalog.disabled.contains(&store),
                supported: store.supported_on(os),
                games: entries
                    .iter()
                    .filter(|entry| entry.store == store && !entry.hidden)
                    .count(),
            })
            .collect()
    }

    pub fn set_store_enabled(&self, store: Store, enabled: bool) -> Result<()> {
        self.update(|catalog| {
            if enabled {
                catalog.disabled.remove(&store);
            } else {
                catalog.disabled.insert(store);
            }
            Ok(())
        })
    }

    /// What to start for a game, checked again right before it is used.
    pub fn launch_target(&self, id: Uuid) -> Result<(Launch, Store)> {
        let catalog = self.catalog();
        let (launch, store) = if let Some(entry) = catalog.games.iter().find(|entry| entry.id == id)
        {
            if !entry.installed {
                return Err(invalid("this game is no longer installed").into());
            }
            (entry.found.launch.clone(), entry.found.store)
        } else if let Some(program) = catalog.programs.iter().find(|program| program.id == id) {
            (program_found(program).launch, Store::Custom)
        } else {
            return Err(invalid("game ID was not found in the library").into());
        };
        check_launch(&launch)?;
        Ok((launch, store))
    }

    /// The game's install folder, if it still exists.
    pub fn install_folder(&self, id: Uuid) -> Result<PathBuf> {
        self.entries()
            .into_iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.install_path)
            .filter(|path| path.is_dir())
            .ok_or_else(|| invalid("the install folder was not found").into())
    }

    pub fn record_play(&self, id: Uuid) -> Result<()> {
        self.change_meta(id, |meta| {
            meta.last_played = Some(crate::now_unix());
            meta.play_count = meta.play_count.saturating_add(1);
        })
    }

    pub fn set_favorite(&self, id: Uuid, favorite: bool) -> Result<()> {
        self.change_meta(id, |meta| meta.favorite = favorite)
    }

    pub fn set_hidden(&self, id: Uuid, hidden: bool) -> Result<()> {
        self.change_meta(id, |meta| meta.hidden = hidden)
    }

    /// Renames the game in GameStrike only. An empty title goes back to the store's title.
    pub fn rename(&self, id: Uuid, title: &str) -> Result<String> {
        let clean = if title.trim().is_empty() {
            None
        } else {
            Some(
                crate::clean_title(title)
                    .ok_or_else(|| invalid("enter a title with 1 to 80 characters"))?,
            )
        };
        let shown = clean.clone();
        self.change_meta(id, |meta| meta.title = clean)?;
        Ok(shown.unwrap_or_else(|| {
            self.entries()
                .into_iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.store_title)
                .unwrap_or_default()
        }))
    }

    /// The PNG to show: the player's cover, else the store's cached art.
    pub fn cover_file(&self, id: Uuid) -> Option<PathBuf> {
        cover_from(&self.game_dir(id))
    }

    /// Sets the player's cover from an image file, decoded with the usual limits. The previous one
    /// is kept for Undo.
    pub fn set_cover(&self, id: Uuid, image: &Path) -> Result<()> {
        if !self.contains(id) {
            return Err(invalid("game ID was not found in the library").into());
        }
        let png = cover_png_from_file(image)?;
        let folder = self.game_dir(id);
        fs::create_dir_all(&folder)?;
        let current = folder.join(CUSTOM_COVER_NAME);
        if regular_file(&current) {
            fs::rename(&current, folder.join(PREVIOUS_COVER_NAME))?;
        }
        write_file_atomic(&current, &png)
    }

    pub fn undo_cover(&self, id: Uuid) -> Result<()> {
        let folder = self.game_dir(id);
        let current = folder.join(CUSTOM_COVER_NAME);
        let previous = folder.join(PREVIOUS_COVER_NAME);
        if !self.contains(id) || !regular_file(&previous) {
            return Err(invalid("there is no earlier cover to bring back").into());
        }
        if regular_file(&current) {
            let swap = folder.join(format!("cover-{}.tmp", Uuid::new_v4().simple()));
            fs::rename(&current, &swap)?;
            fs::rename(&previous, &current)?;
            fs::rename(&swap, &previous)?;
        } else {
            fs::rename(&previous, &current)?;
        }
        Ok(())
    }

    /// Back to the store's art (or the generated cover). The player's cover is kept for Undo.
    pub fn reset_cover(&self, id: Uuid) -> Result<()> {
        if !self.contains(id) {
            return Err(invalid("game ID was not found in the library").into());
        }
        let folder = self.game_dir(id);
        let current = folder.join(CUSTOM_COVER_NAME);
        if regular_file(&current) {
            fs::rename(&current, folder.join(PREVIOUS_COVER_NAME))?;
        }
        Ok(())
    }

    /// Adds a program the player picked (a native picker or the command line supplies the path).
    pub fn add_program(&self, path: &Path, title: Option<&str>) -> Result<Uuid> {
        if !path.is_absolute() || !(regular_file(path) || path.is_dir()) {
            return Err(invalid("choose an existing program, shortcut, or app").into());
        }
        let title = title
            .and_then(crate::clean_title)
            .or_else(|| program_title(path))
            .ok_or_else(|| invalid("the program has no usable name"))?;
        let id = Uuid::new_v4();
        let program = CustomProgram {
            id,
            title,
            path: path.to_path_buf(),
            args: Vec::new(),
            working_dir: None,
            added_at: crate::now_unix(),
        };
        self.update(|catalog| {
            if catalog.programs.len() >= 1000 {
                return Err(invalid("there are too many custom programs").into());
            }
            catalog.programs.push(program);
            Ok(id)
        })
    }

    /// Sets a custom program's arguments and, optionally, its working folder.
    pub fn set_program(
        &self,
        id: Uuid,
        args: Vec<String>,
        working_dir: Option<PathBuf>,
    ) -> Result<()> {
        check_arguments(&args)?;
        if working_dir
            .as_deref()
            .is_some_and(|dir| !dir.is_absolute() || !dir.is_dir())
        {
            return Err(invalid("the working folder was not found").into());
        }
        self.update(|catalog| {
            let program = catalog
                .programs
                .iter_mut()
                .find(|program| program.id == id)
                .ok_or_else(|| invalid("only custom programs have arguments"))?;
            program.args = args;
            if working_dir.is_some() {
                program.working_dir = working_dir;
            }
            Ok(())
        })
    }

    /// Removes a custom program from GameStrike (the program itself is not touched), with its data.
    pub fn remove_program(&self, id: Uuid) -> Result<()> {
        self.update(|catalog| {
            let before = catalog.programs.len();
            catalog.programs.retain(|program| program.id != id);
            if catalog.programs.len() == before {
                return Err(invalid("only custom programs can be removed").into());
            }
            Ok(())
        })?;
        let folder = self.game_dir(id);
        if folder.is_dir() {
            fs::remove_dir_all(folder)?;
        }
        Ok(())
    }
}

const CUSTOM_COVER_NAME: &str = "cover.png";
const PREVIOUS_COVER_NAME: &str = "cover-previous.png";

/// A store game's cover in a library at `library_root`: the player's, else the store's cached art.
pub fn cover_file_at(library_root: &Path, id: Uuid) -> Option<PathBuf> {
    cover_from(&library_root.join("stores").join(id.to_string()))
}

fn cover_from(folder: &Path) -> Option<PathBuf> {
    [CUSTOM_COVER_NAME, STORE_COVER]
        .into_iter()
        .map(|name| folder.join(name))
        .find(|path| regular_file(path))
}

fn program_found(program: &CustomProgram) -> Found {
    Found {
        store: Store::Custom,
        store_id: program.id.to_string(),
        title: program.title.clone(),
        install_path: program.path.parent().map(Path::to_path_buf),
        launch: Launch::Program {
            path: program.path.clone(),
            args: program.args.clone(),
            working_dir: program.working_dir.clone(),
        },
        cover_source: None,
    }
}

/// A name for a program from its file name, without the extension (`.exe`, `.lnk`, `.app`, ...).
fn program_title(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(clean_store_title)
}

fn modified_millis(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_millis() as u64)
}

fn write_json(path: &Path, value: &impl Serialize, limit: u64) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("store data is too large").into());
    }
    write_file_atomic(path, &bytes)
}

fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let mut file = new_file(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY_FOLDERS: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"ROOT"
		"label"		""
		"apps"
		{
			"10"		"1234"
		}
	}
	"1"
	{
		"path"		"SECOND"
	}
}
"#;

    fn acf(appid: &str, name: &str, dir: &str, flags: u32) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{dir}\"\n}}\n"
        )
    }

    fn escaped(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\")
    }

    #[test]
    fn vdf_parses_nesting_escapes_comments_and_bare_tokens() {
        let document = parse_vdf(
            "// comment\n\"a\" { \"path\" \"C:\\\\Games\\\\Steam\" \"quote\" \"say \\\"hi\\\"\" b { c 1 } }",
        )
        .unwrap();
        let a = document.get("A").unwrap();
        assert_eq!(a.text("path"), Some("C:\\Games\\Steam"));
        assert_eq!(a.text("quote"), Some("say \"hi\""));
        assert_eq!(a.get("b").unwrap().text("c"), Some("1"));
        assert!(parse_vdf("\"a\" {").is_err());
        assert!(parse_vdf("\"a\"").is_err());
        assert!(parse_vdf("\"a\" \"unterminated").is_err());
        let deep = "\"a\" {".repeat(40) + &"}".repeat(40);
        assert!(parse_vdf(&deep).is_err(), "nesting is limited");
    }

    #[test]
    fn steam_finds_installed_games_in_every_library_and_skips_tools() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Steam");
        let second = dir.path().join("Second Library");
        for library in [&root, &second] {
            fs::create_dir_all(library.join("steamapps/common")).unwrap();
        }
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            LIBRARY_FOLDERS
                .replace("ROOT", &escaped(&root))
                .replace("SECOND", &escaped(&second)),
        )
        .unwrap();
        let manifests = [
            (
                &root,
                acf("440", "Synthetic Fortress (Synthetic Test)", "Fortress", 4),
            ),
            (
                &second,
                acf("620", "Synthetic Portal ™ (Synthetic Test)", "Portal", 4),
            ),
            (
                &root,
                acf("700", "Half Downloaded (Synthetic Test)", "Half", 1026 & !4),
            ),
            (
                &root,
                acf("228980", "Steamworks Common Redistributables", "Redist", 4),
            ),
            (&root, acf("999", "Proton 9.0", "Proton", 4)),
        ];
        for (library, text) in &manifests {
            let id = text.split('"').nth(5).unwrap();
            fs::write(
                library.join(format!("steamapps/appmanifest_{id}.acf")),
                text,
            )
            .unwrap();
        }
        let flat = root.join("appcache/librarycache");
        fs::create_dir_all(flat.join("620/abcdef")).unwrap();
        fs::write(flat.join("440_library_600x900.jpg"), b"jpg").unwrap();
        fs::write(flat.join("620/abcdef/library_600x900.jpg"), b"jpg").unwrap();

        let env = ScanEnv {
            os: HostOs::Unix,
            registry: &NoRegistry,
            home: None,
            program_data: None,
            steam_root: Some(root.clone()),
            all_platforms: false,
        };
        let found = scan(Store::Steam, &env);
        let ids: Vec<&str> = found.iter().map(|game| game.store_id.as_str()).collect();
        assert_eq!(
            ids,
            ["440", "620"],
            "only installed games, no tools or runtimes"
        );
        assert_eq!(
            found[1].title, "Synthetic Portal (Synthetic Test)",
            "trademark signs dropped"
        );
        assert_eq!(
            found[0].launch,
            Launch::Uri {
                uri: "steam://rungameid/440".to_owned()
            }
        );
        assert_eq!(
            found[1].install_path.as_deref(),
            Some(second.join("steamapps/common/Portal").as_path())
        );
        assert_eq!(
            found[0].cover_source.as_deref(),
            Some(flat.join("440_library_600x900.jpg").as_path())
        );
        assert_eq!(
            found[1].cover_source.as_deref(),
            Some(flat.join("620/abcdef/library_600x900.jpg").as_path()),
            "the newer per-app cache layout"
        );
    }

    #[test]
    fn steam_reads_the_older_library_list_format() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Steam");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        let other = dir.path().join("Other");
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"LibraryFolders\" {{ \"TimeNextStatsReport\" \"1\" \"1\" \"{}\" }}",
                escaped(&other)
            ),
        )
        .unwrap();
        let libraries = steam_libraries(&root);
        assert_eq!(libraries, [root.clone(), other]);
    }

    #[test]
    fn steam_is_found_through_the_registry_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Steam");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        let registry = SnapshotRegistry::from_json(&format!(
            r#"{{"HKCU\\Software\\Valve\\Steam": {{"SteamPath": "{}"}}}}"#,
            root.to_string_lossy().replace('\\', "/")
        ))
        .unwrap();
        let env = ScanEnv {
            os: HostOs::Windows,
            registry: &registry,
            home: None,
            program_data: None,
            steam_root: None,
            all_platforms: false,
        };
        assert_eq!(steam_roots(&env).len(), 1);
    }

    #[test]
    fn epic_manifests_become_launch_links_and_add_ons_are_skipped() {
        let game = epic_game(
            r#"{"DisplayName":"Synthetic Quest (Synthetic Test)","AppName":"SynthQuest","InstallLocation":"C:\\Games\\Synth","LaunchExecutable":"Synth.exe","CatalogNamespace":"abc123","CatalogItemId":"0f1e2d","MainGameAppName":"SynthQuest","AppCategories":["public","games","applications"],"bIsIncompleteInstall":false}"#,
        )
        .unwrap();
        assert_eq!(game.store_id, "SynthQuest");
        assert_eq!(
            game.launch,
            Launch::Uri {
                uri: "com.epicgames.launcher://apps/abc123%3A0f1e2d%3ASynthQuest?action=launch&silent=true".to_owned()
            }
        );
        let short =
            epic_game(r#"{"DisplayName":"Old","AppName":"Old","InstallLocation":"D:\\Old"}"#)
                .unwrap();
        assert_eq!(
            short.launch,
            Launch::Uri {
                uri: "com.epicgames.launcher://apps/Old?action=launch&silent=true".to_owned()
            }
        );
        for skipped in [
            r#"{"AppName":"Dlc","MainGameAppName":"SynthQuest","InstallLocation":"C:\\x"}"#,
            r#"{"AppName":"Engine","AppCategories":["engines"],"InstallLocation":"C:\\x"}"#,
            r#"{"AppName":"Half","bIsIncompleteInstall":true,"InstallLocation":"C:\\x"}"#,
            r#"{"AppName":"bad name;calc","InstallLocation":"C:\\x"}"#,
            r#"{"AppName":"NoFolder"}"#,
            "not json",
        ] {
            assert!(epic_game(skipped).is_none(), "{skipped}");
        }
    }

    fn snapshot(entries: &[(&str, &[(&str, &str)])]) -> SnapshotRegistry {
        let map: BTreeMap<String, BTreeMap<String, String>> = entries
            .iter()
            .map(|(key, values)| {
                (
                    (*key).to_owned(),
                    values
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                        .collect(),
                )
            })
            .collect();
        SnapshotRegistry::from_json(&serde_json::to_string(&map).unwrap()).unwrap()
    }

    fn windows_env(registry: &dyn Registry) -> ScanEnv<'_> {
        ScanEnv {
            os: HostOs::Windows,
            registry,
            home: None,
            program_data: None,
            steam_root: None,
            all_platforms: false,
        }
    }

    #[test]
    fn registry_snapshot_lists_subkeys_case_insensitively() {
        let registry = snapshot(&[
            ("HKLM\\SOFTWARE\\Vendor\\Games\\100", &[("Name", "A")]),
            (
                "HKLM\\SOFTWARE\\Vendor\\Games\\200\\Deeper",
                &[("Name", "B")],
            ),
        ]);
        assert_eq!(
            registry.subkeys(Hive::LocalMachine, "software\\vendor\\games"),
            ["100", "200"]
        );
        assert_eq!(
            registry
                .string(Hive::LocalMachine, "SOFTWARE\\VENDOR\\GAMES\\100", "name")
                .as_deref(),
            Some("A")
        );
        assert!(
            registry
                .subkeys(Hive::CurrentUser, "SOFTWARE\\Vendor\\Games")
                .is_empty()
        );
    }

    #[test]
    fn gog_games_launch_through_galaxy_or_their_own_program() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Synthetic Witcher");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("game.exe"), b"").unwrap();
        let galaxy = dir.path().join("Galaxy");
        fs::create_dir_all(&galaxy).unwrap();
        fs::write(galaxy.join("GalaxyClient.exe"), b"").unwrap();
        let game_path = game.to_string_lossy().into_owned();
        let exe = game.join("game.exe").to_string_lossy().into_owned();
        let galaxy_path = galaxy.to_string_lossy().into_owned();
        let games: &[(&str, &[(&str, &str)])] = &[
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\1207658924",
                &[
                    ("gameName", "Synthetic Witcher (Synthetic Test)"),
                    ("path", &game_path),
                    ("exe", &exe),
                    ("launchParam", "-skip \"intro movie\""),
                ],
            ),
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\1500000000",
                &[
                    ("gameName", "A Soundtrack"),
                    ("path", &game_path),
                    ("dependsOn", "1207658924"),
                ],
            ),
        ];
        let found = scan_gog(&windows_env(&snapshot(games)));
        assert_eq!(found.len(), 1, "add-ons are skipped");
        match &found[0].launch {
            Launch::Program { path, args, .. } => {
                assert_eq!(path, &game.join("game.exe"));
                assert_eq!(args, &["-skip", "intro movie"]);
            }
            other => panic!("{other:?}"),
        }
        let client: &[(&str, &str)] = &[("client", &galaxy_path)];
        let mut with_galaxy = games.to_vec();
        with_galaxy.push((
            "HKLM\\SOFTWARE\\WOW6432Node\\GOG.com\\GalaxyClient\\paths",
            client,
        ));
        let found = scan_gog(&windows_env(&snapshot(&with_galaxy)));
        match &found[0].launch {
            Launch::Program { path, args, .. } => {
                assert_eq!(path, &galaxy.join("GalaxyClient.exe"));
                assert_eq!(args[0], "/command=runGame");
                assert_eq!(args[1], "/gameId=1207658924");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ubisoft_games_take_their_name_from_the_uninstall_entry() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Synthetic Creed");
        fs::create_dir_all(&game).unwrap();
        let install = format!("{}/", game.to_string_lossy().replace('\\', "/"));
        let registry = snapshot(&[
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\Ubisoft\\Launcher\\Installs\\720",
                &[("InstallDir", &install)],
            ),
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 720",
                &[("DisplayName", "Synthetic Creed (Synthetic Test)")],
            ),
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\Ubisoft\\Launcher\\Installs\\721",
                &[("InstallDir", "Z:/missing")],
            ),
        ]);
        let found = scan_ubisoft(&windows_env(&registry));
        assert_eq!(found.len(), 1, "missing folders are skipped");
        assert_eq!(found[0].title, "Synthetic Creed (Synthetic Test)");
        assert_eq!(
            found[0].launch,
            Launch::Uri {
                uri: "uplay://launch/720/0".to_owned()
            }
        );
    }

    #[test]
    fn ea_games_need_a_content_id_from_their_installer_data() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Synthetic Battlefront");
        fs::create_dir_all(game.join("__Installer")).unwrap();
        fs::write(
            game.join("__Installer/installerdata.xml"),
            "<DiPManifest><contentIDs><contentID>1234567</contentID><contentID>OFB-EAST:7654</contentID></contentIDs></DiPManifest>",
        )
        .unwrap();
        let bare = dir.path().join("No Data");
        fs::create_dir_all(&bare).unwrap();
        let game_path = game.to_string_lossy().into_owned();
        let bare_path = bare.to_string_lossy().into_owned();
        let registry = snapshot(&[
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\EA Games\\Synthetic Battlefront",
                &[
                    ("Install Dir", &game_path),
                    ("DisplayName", "Synthetic Battlefront (Synthetic Test)"),
                ],
            ),
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\EA Games\\No Data",
                &[("Install Dir", &bare_path)],
            ),
            (
                "HKLM\\SOFTWARE\\WOW6432Node\\EA Games\\EA Desktop",
                &[("Install Dir", &game_path)],
            ),
        ]);
        let found = scan_ea(&windows_env(&registry));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].store_id, "1234567");
        assert_eq!(
            found[0].launch,
            Launch::Uri {
                uri: "origin2://game/launch?offerIds=1234567,OFB-EAST:7654&autoDownload=1"
                    .to_owned()
            }
        );
        assert!(ea_content_ids("<contentID>bad id</contentID>").is_empty());
    }

    #[test]
    fn battlenet_games_come_from_blizzard_uninstall_entries() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Synthetic Diablo");
        let client = dir.path().join("Battle.net");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&client).unwrap();
        fs::write(client.join("Battle.net.exe"), b"").unwrap();
        let game_path = game.to_string_lossy().into_owned();
        let client_path = client.to_string_lossy().into_owned();
        let uninstall =
            "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
        let fenris = format!("{uninstall}\\Diablo IV");
        let bnet = format!("{uninstall}\\Battle.net");
        let other = format!("{uninstall}\\Other");
        let registry = snapshot(&[
            (
                &fenris,
                &[
                    ("DisplayName", "Synthetic Diablo (Synthetic Test)"),
                    ("Publisher", "Blizzard Entertainment"),
                    ("InstallLocation", &game_path),
                    (
                        "UninstallString",
                        "\"C:\\ProgramData\\Battle.net\\Agent\\Blizzard Uninstaller.exe\" --lang=enUS --uid=fenris --displayname=\"Diablo IV\"",
                    ),
                ],
            ),
            (
                &bnet,
                &[
                    ("DisplayName", "Battle.net"),
                    ("Publisher", "Blizzard Entertainment"),
                    ("InstallLocation", &client_path),
                ],
            ),
            (
                &other,
                &[
                    ("DisplayName", "Not Blizzard"),
                    ("Publisher", "Someone"),
                    ("InstallLocation", &game_path),
                    ("UninstallString", "x --uid=wow"),
                ],
            ),
        ]);
        let found = scan_battlenet(&windows_env(&registry));
        assert_eq!(found.len(), 1);
        match &found[0].launch {
            Launch::Program { path, args, .. } => {
                assert_eq!(path, &client.join("Battle.net.exe"));
                assert_eq!(args, &["--exec=launch Fen"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            battlenet_uid("x --uid=wow_classic --y"),
            Some("wow_classic".to_owned())
        );
        assert_eq!(battlenet_uid("no uid"), None);
    }

    #[test]
    fn microsoft_store_games_need_a_game_config_and_launch_through_the_apps_folder() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Synthetic Forza");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("MicrosoftGame.config"),
            r#"<?xml version="1.0"?><Game configVersion="1"><Identity Name="SynthForza"/><ShellVisuals DefaultDisplayName="Synthetic Horizon &amp; Friends (Synthetic Test)" Square480x480Logo="Logo480.png"/></Game>"#,
        )
        .unwrap();
        fs::write(game.join("Logo480.png"), b"png").unwrap();
        fs::write(
            game.join("AppxManifest.xml"),
            r#"<Package><Identity Name="SynthForza" Publisher="CN=x"/><Applications><Application Id="Game" Executable="game.exe"/></Applications></Package>"#,
        )
        .unwrap();
        let app = dir.path().join("Calculator");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("AppxManifest.xml"), r#"<Application Id="App"/>"#).unwrap();
        let packages = "HKCU\\Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages";
        let game_key = format!("{packages}\\SynthForza_1.0.0.0_x64__8wekyb3d8bbwe");
        let app_key = format!("{packages}\\Calculator_1.0.0.0_x64__8wekyb3d8bbwe");
        let game_path = game.to_string_lossy().into_owned();
        let app_path = app.to_string_lossy().into_owned();
        let registry = snapshot(&[
            (&game_key, &[("PackageRootFolder", &game_path)]),
            (&app_key, &[("PackageRootFolder", &app_path)]),
        ]);
        let found = scan_xbox(&windows_env(&registry));
        assert_eq!(found.len(), 1, "apps without a game config are not games");
        assert_eq!(
            found[0].title,
            "Synthetic Horizon & Friends (Synthetic Test)"
        );
        assert_eq!(found[0].store_id, "SynthForza_8wekyb3d8bbwe");
        assert_eq!(
            found[0].launch,
            Launch::Uri {
                uri: "shell:AppsFolder\\SynthForza_8wekyb3d8bbwe!Game".to_owned()
            }
        );
        assert_eq!(
            found[0].cover_source.as_deref(),
            Some(game.join("Logo480.png").as_path())
        );
        assert_eq!(
            xml_attribute("<A x=\"1\" ax=\"2\"/>", "A", "x").as_deref(),
            Some("1")
        );
        assert_eq!(
            xml_attribute("<AB x=\"1\"/><A x=\"2\"/>", "A", "x").as_deref(),
            Some("2")
        );
    }

    #[test]
    fn windows_only_stores_are_not_scanned_elsewhere_unless_fixtures_say_so() {
        let registry = snapshot(&[(
            "HKLM\\SOFTWARE\\WOW6432Node\\Ubisoft\\Launcher\\Installs\\1",
            &[("InstallDir", "/tmp")],
        )]);
        let mut env = windows_env(&registry);
        env.os = HostOs::Unix;
        assert!(scan(Store::Ubisoft, &env).is_empty());
        assert!(
            Store::Steam.supported_on(HostOs::MacOs) && Store::Custom.supported_on(HostOs::Unix)
        );
        assert!(!Store::Epic.supported_on(HostOs::Unix));
        env.all_platforms = true;
        assert_eq!(
            scan(Store::Ubisoft, &env).len(),
            usize::from(Path::new("/tmp").is_dir())
        );
    }

    #[test]
    fn only_store_links_with_safe_characters_are_launched() {
        for good in [
            "steam://rungameid/440",
            "com.epicgames.launcher://apps/a%3Ab%3Ac?action=launch&silent=true",
            "uplay://launch/720/0",
            "origin2://game/launch?offerIds=1,OFB-EAST:2&autoDownload=1",
            "battlenet://Fen",
            "shell:AppsFolder\\Name_pub!Game",
        ] {
            assert!(check_launch_uri(good).is_ok(), "{good}");
        }
        for bad in [
            "https://example.com",
            "file:///C:/Windows/System32/calc.exe",
            "steam://rungameid/440 & calc",
            "steam://rungameid/440\"",
            "shell:AppsFolder\\x y",
            "javascript:alert(1)",
        ] {
            assert!(check_launch_uri(bad).is_err(), "{bad}");
        }
        assert!(
            check_launch(&Launch::Program {
                path: PathBuf::from("relative.exe"),
                args: Vec::new(),
                working_dir: None
            })
            .is_err()
        );
        assert!(check_arguments(&["a\0b".to_owned()]).is_err());
        let many: Vec<String> = (0..33).map(|_| "x".to_owned()).collect();
        assert!(check_arguments(&many).is_err());
        assert_eq!(
            split_arguments("  -w \"my save\" --fast  "),
            ["-w", "my save", "--fast"]
        );
        assert_eq!(split_arguments("\"\""), [""]);
    }

    #[test]
    fn store_ids_are_stable_and_distinct() {
        let a = store_game_id(Store::Steam, "440");
        assert_eq!(a, store_game_id(Store::Steam, "440"));
        assert_ne!(a, store_game_id(Store::Epic, "440"));
        let program = Uuid::new_v4();
        assert_eq!(store_game_id(Store::Custom, &program.to_string()), program);
    }

    fn steam_fixture(dir: &Path, games: &[(&str, &str)]) -> PathBuf {
        let root = dir.join("steam");
        fs::create_dir_all(root.join("steamapps/common")).unwrap();
        for entry in fs::read_dir(root.join("steamapps")).unwrap().flatten() {
            if entry.path().is_file() {
                fs::remove_file(entry.path()).unwrap();
            }
        }
        for (id, name) in games {
            fs::write(
                root.join(format!("steamapps/appmanifest_{id}.acf")),
                acf(id, name, name, 4),
            )
            .unwrap();
        }
        root
    }

    #[test]
    fn refresh_merges_marks_uninstalled_games_and_keeps_player_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = steam_fixture(dir.path(), &[("10", "Keep Me"), ("20", "Forget Me")]);
        let library = StoreLibrary::open(dir.path()).unwrap();
        let env = ScanEnv {
            os: HostOs::Unix,
            registry: &NoRegistry,
            home: None,
            program_data: None,
            steam_root: Some(root.clone()),
            all_platforms: false,
        };
        assert_eq!(library.refresh(&env).unwrap(), 2);
        let keep = store_game_id(Store::Steam, "10");
        library.set_favorite(keep, true).unwrap();
        library.record_play(keep).unwrap();
        assert_eq!(library.rename(keep, "My Name").unwrap(), "My Name");
        steam_fixture(dir.path(), &[]);
        assert_eq!(library.refresh(&env).unwrap(), 0);
        let entries = library.entries();
        assert_eq!(
            entries.len(),
            1,
            "a game the player never touched disappears"
        );
        assert_eq!(entries[0].id, keep);
        assert!(!entries[0].installed && entries[0].favorite && entries[0].play_count == 1);
        assert_eq!(entries[0].title, "My Name");
        assert!(
            library.launch_target(keep).is_err(),
            "uninstalled games do not launch"
        );
        steam_fixture(dir.path(), &[("10", "Keep Me")]);
        library.refresh(&env).unwrap();
        let entry = &library.entries()[0];
        assert!(
            entry.installed && entry.favorite,
            "reinstalling keeps the player's data"
        );
        assert_eq!(
            library.rename(keep, "  ").unwrap(),
            "Keep Me",
            "an empty name goes back to the store's"
        );
        let (launch, store) = library.launch_target(keep).unwrap();
        assert_eq!(store, Store::Steam);
        assert_eq!(
            launch,
            Launch::Uri {
                uri: "steam://rungameid/10".to_owned()
            }
        );
    }

    #[test]
    fn turned_off_stores_are_not_scanned_or_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = steam_fixture(dir.path(), &[("10", "Game")]);
        let library = StoreLibrary::open(dir.path()).unwrap();
        let env = ScanEnv {
            os: HostOs::Unix,
            registry: &NoRegistry,
            home: None,
            program_data: None,
            steam_root: Some(root),
            all_platforms: false,
        };
        library.refresh(&env).unwrap();
        library.set_store_enabled(Store::Steam, false).unwrap();
        assert!(library.entries().is_empty());
        let statuses = library.statuses(HostOs::Unix);
        let steam = statuses
            .iter()
            .find(|status| status.key == "steam")
            .unwrap();
        assert!(!steam.enabled && steam.supported && steam.games == 0);
        assert!(
            !statuses
                .iter()
                .find(|status| status.key == "epic")
                .unwrap()
                .supported
        );
        library.set_store_enabled(Store::Steam, true).unwrap();
        assert_eq!(library.entries().len(), 1);
    }

    #[test]
    fn custom_programs_keep_arguments_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("Synthetic Tool.exe");
        fs::write(&program, b"").unwrap();
        let library = StoreLibrary::open(dir.path()).unwrap();
        assert!(
            library
                .add_program(Path::new("relative.exe"), None)
                .is_err()
        );
        let id = library.add_program(&program, None).unwrap();
        let entry = library.entries().into_iter().next().unwrap();
        assert_eq!(entry.title, "Synthetic Tool");
        assert_eq!(entry.store, Store::Custom);
        library
            .set_program(
                id,
                split_arguments("--windowed -x 2"),
                Some(dir.path().to_path_buf()),
            )
            .unwrap();
        match library.launch_target(id).unwrap().0 {
            Launch::Program {
                path,
                args,
                working_dir,
            } => {
                assert_eq!(path, program);
                assert_eq!(args, ["--windowed", "-x", "2"]);
                assert_eq!(working_dir.as_deref(), Some(dir.path()));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            library
                .set_program(id, Vec::new(), Some(dir.path().join("missing")))
                .is_err()
        );
        library.set_hidden(id, true).unwrap();
        assert!(library.entries()[0].hidden);
        library.remove_program(id).unwrap();
        assert!(library.entries().is_empty());
        assert!(library.remove_program(id).is_err());
    }

    #[test]
    fn store_covers_are_decoded_cached_and_replaceable() {
        let dir = tempfile::tempdir().unwrap();
        let root = steam_fixture(dir.path(), &[("10", "Covered")]);
        let cache = root.join("appcache/librarycache");
        fs::create_dir_all(&cache).unwrap();
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(30, 45)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        fs::write(cache.join("10_library_600x900.jpg"), &png).unwrap();
        let library = StoreLibrary::open(dir.path()).unwrap();
        let env = ScanEnv {
            os: HostOs::Unix,
            registry: &NoRegistry,
            home: None,
            program_data: None,
            steam_root: Some(root),
            all_platforms: false,
        };
        library.refresh(&env).unwrap();
        let id = store_game_id(Store::Steam, "10");
        assert_eq!(library.entries()[0].cover, Some("store"));
        assert!(library.cover_file(id).unwrap().ends_with(STORE_COVER));
        let custom = dir.path().join("mine.png");
        fs::write(&custom, &png).unwrap();
        library.set_cover(id, &custom).unwrap();
        assert_eq!(library.entries()[0].cover, Some("custom"));
        library.reset_cover(id).unwrap();
        assert_eq!(library.entries()[0].cover, Some("store"));
        assert!(library.entries()[0].cover_undo);
        library.undo_cover(id).unwrap();
        assert_eq!(library.entries()[0].cover, Some("custom"));
    }

    #[test]
    fn fixtures_scan_every_store_from_one_folder() {
        let dir = tempfile::tempdir().unwrap();
        steam_fixture(dir.path(), &[("10", "Synthetic Steam Game")]);
        let manifests = dir
            .path()
            .join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests");
        fs::create_dir_all(&manifests).unwrap();
        let epic_dir = dir.path().join("EpicGame");
        fs::create_dir_all(&epic_dir).unwrap();
        fs::write(
            manifests.join("a.item"),
            format!(
                r#"{{"DisplayName":"Synthetic Epic Game","AppName":"SynthEpic","InstallLocation":"{}"}}"#,
                escaped(&epic_dir)
            ),
        )
        .unwrap();
        let env = ScanEnv::for_fixtures(dir.path(), &NoRegistry);
        assert_eq!(scan(Store::Steam, &env).len(), 1);
        assert_eq!(scan(Store::Epic, &env).len(), 1);
    }
}
