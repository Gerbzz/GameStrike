#![forbid(unsafe_code)]

//! Import-only foundation. This crate never interprets imported HTML or JavaScript.

pub mod application;
pub mod music;
pub mod native_capture;
#[cfg(feature = "native-ui")]
pub mod native_ui;
pub mod stores;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const MAX_MANIFEST_BYTES: u64 = 8192;
const MAX_PATH_BYTES: usize = 240;
const MAX_EXPANSION_RATIO: u64 = 200;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    SingleHtml,
    Zip,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Game {
    pub id: Uuid,
    pub title: String,
    pub source_kind: SourceKind,
    pub entry: String,
    pub controller_support: String,
    pub network_requirement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    /// Seconds since the Unix epoch. Absent in manifests written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<u64>,
    /// Where the game came from, as the player wrote it (for example "itch.io"). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Marked as a favorite in the launcher.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
}

pub struct ImportOutcome {
    pub game: Game,
    pub already_present: bool,
    /// True when a removed copy of this exact file left its saves behind and they were reattached.
    pub restored_saves: bool,
}

/// One game as the launcher shows it: its manifest plus locally observed state.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LibraryEntry {
    #[serde(flatten)]
    pub game: Game,
    /// False when `content/index.html` is gone, for example after manual file cleanup.
    pub files_ok: bool,
    pub last_played: Option<u64>,
    pub play_count: u32,
    /// Bytes in this game's browser profile (saves, caches, and WebView2 state).
    pub save_bytes: u64,
    /// Which cover the launcher shows: "custom" (set by the player), "auto" (from an image in the
    /// game's own files), or none, in which case the launcher draws one from the title.
    pub cover: Option<&'static str>,
    /// Changes whenever the cover changes, so the launcher can ask for the new image.
    pub cover_version: u64,
    /// A previous custom cover can be brought back.
    pub cover_undo: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LibraryScan {
    pub games: Vec<LibraryEntry>,
    /// Folders under `games/` that could not be read as a game. They are left untouched.
    pub problems: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Activity {
    last_played: Option<u64>,
    play_count: u32,
}

#[derive(Serialize, Deserialize)]
struct RetainedSaves {
    source_kind: SourceKind,
    source_sha256: String,
    title: String,
}

/// Exclusive per-game lock held by the player while a game runs. Released by the OS on exit.
pub struct GameLock {
    _file: File,
}

pub struct Library {
    root: PathBuf,
    import_lock: Mutex<()>,
}

const MAX_TITLE_CHARS: usize = 80;
const MAX_PROFILE_WALK: usize = 50_000;

impl Library {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("games"))?;
        fs::create_dir_all(root.join("incoming"))?;
        Ok(Self {
            root,
            import_lock: Mutex::new(()),
        })
    }

    pub fn import(&self, source: impl AsRef<Path>) -> Result<Game> {
        Ok(self.import_with_result(source)?.game)
    }

    pub fn import_with_result(&self, source: impl AsRef<Path>) -> Result<ImportOutcome> {
        let source = source.as_ref();
        let metadata = fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("source must be a regular file").into());
        }
        if metadata.len() == 0 || metadata.len() > MAX_SOURCE_BYTES {
            return Err(invalid("source is empty or exceeds 256 MiB").into());
        }

        let kind = match source.extension().and_then(|value| value.to_str()) {
            Some(value)
                if value.eq_ignore_ascii_case("html") || value.eq_ignore_ascii_case("htm") =>
            {
                SourceKind::SingleHtml
            }
            Some(value) if value.eq_ignore_ascii_case("zip") => SourceKind::Zip,
            _ => return Err(invalid("supported sources are .html, .htm, and .zip").into()),
        };
        // Keep the duplicate decision and final install together for this library instance.
        let _guard = self
            .import_lock
            .lock()
            .map_err(|_| invalid("import lock is unavailable"))?;
        let source_sha256 = hash_source(source, metadata.len())?;
        let games = self.list()?;
        if let Some(game) = games.iter().find(|game| {
            game.source_kind == kind
                && game.source_sha256.as_deref() == Some(source_sha256.as_str())
        }) {
            return Ok(ImportOutcome {
                game: game.clone(),
                already_present: true,
                restored_saves: false,
            });
        }
        let legacy_games: Vec<_> = games
            .into_iter()
            .filter(|game| game.source_kind == kind && game.source_sha256.is_none())
            .collect();
        // Re-adding the exact file of a game removed with "keep saves" reuses its ID, which
        // restores the same browser origin and profile.
        let retained = self.find_retained(&kind, &source_sha256);
        let id = retained.unwrap_or_else(Uuid::new_v4);
        let game = Game {
            id,
            title: display_title(source),
            source_kind: kind.clone(),
            entry: "index.html".to_owned(),
            controller_support: "unknown".to_owned(),
            network_requirement: "unknown".to_owned(),
            source_sha256: Some(source_sha256.clone()),
            added_at: Some(now_unix()),
            source: None,
            favorite: false,
        };
        let stage = self.root.join("incoming").join(id.to_string());
        fs::create_dir(&stage)?;
        let result = (|| -> Result<Option<Game>> {
            let content = stage.join("content");
            fs::create_dir(&content)?;
            match kind {
                SourceKind::SingleHtml => {
                    let mut input = File::open(source)?;
                    let mut output = new_file(&content.join("index.html"))?;
                    let mut total = 0;
                    copy_bounded(&mut input, &mut output, &mut total, MAX_FILE_BYTES)?;
                }
                SourceKind::Zip => import_zip(source, &content)?,
            }
            if hash_source(source, metadata.len())? != source_sha256 {
                return Err(invalid("source changed while importing").into());
            }
            if !legacy_games.is_empty() {
                let staged_digest = hash_content_tree(&content)?;
                for existing in &legacy_games {
                    let old_content = self
                        .root
                        .join("games")
                        .join(existing.id.to_string())
                        .join("content");
                    if hash_content_tree(&old_content).is_ok_and(|digest| digest == staged_digest) {
                        // A pre-fingerprint manifest can be read and re-imported. Record the
                        // exact source hash now so a later keep-saves removal can identify it.
                        let mut existing = existing.clone();
                        existing.source_sha256 = Some(source_sha256.clone());
                        write_json_atomic(
                            &self.game_dir(existing.id).join("game.json"),
                            &existing,
                        )?;
                        return Ok(Some(existing));
                    }
                }
            }
            let mut manifest = new_file(&stage.join("game.json"))?;
            serde_json::to_writer_pretty(&mut manifest, &game)?;
            manifest.sync_all()?;
            drop(manifest);
            fs::rename(&stage, self.root.join("games").join(id.to_string()))?;
            Ok(None)
        })();
        match result {
            Ok(Some(existing)) => {
                // The freshly extracted UUID stage is disposable; the old game and profile stay intact.
                fs::remove_dir_all(&stage)?;
                Ok(ImportOutcome {
                    game: existing,
                    already_present: true,
                    restored_saves: false,
                })
            }
            Ok(None) => {
                // A cover found in the game's own files is optional; a bad one is simply not used.
                let _ = self.ensure_auto_cover(id);
                if retained.is_some() {
                    let _ = fs::remove_file(self.retained_marker(id));
                }
                Ok(ImportOutcome {
                    game,
                    already_present: false,
                    restored_saves: retained.is_some(),
                })
            }
            Err(error) => {
                // Only this call's newly created UUID staging directory is removed.
                let _ = fs::remove_dir_all(&stage);
                Err(error)
            }
        }
    }

    /// Readable games, sorted by title. Unreadable folders are skipped; see [`Library::scan`].
    pub fn list(&self) -> Result<Vec<Game>> {
        Ok(self.read_games()?.0)
    }

    /// Games with local state for the launcher, plus any folders that could not be read.
    pub fn scan(&self) -> Result<LibraryScan> {
        let (games, problems) = self.read_games()?;
        let games = games
            .into_iter()
            .map(|game| {
                let activity = self.read_activity(game.id);
                LibraryEntry {
                    files_ok: self.entry_file(game.id).is_ok(),
                    last_played: activity.last_played,
                    play_count: activity.play_count,
                    save_bytes: directory_size(&self.profile_dir(game.id)),
                    cover: self.cover_kind(game.id),
                    cover_version: self.cover_version(game.id),
                    cover_undo: regular_file(&self.game_dir(game.id).join(PREVIOUS_COVER)),
                    game,
                }
            })
            .collect();
        Ok(LibraryScan { games, problems })
    }

    fn read_games(&self) -> Result<(Vec<Game>, Vec<String>)> {
        let mut games = Vec::new();
        let mut problems = Vec::new();
        for item in fs::read_dir(self.root.join("games"))? {
            let item = item?;
            if !item.file_type()?.is_dir() {
                continue;
            }
            let folder = item.file_name().to_string_lossy().into_owned();
            match read_manifest(&item.path(), &folder) {
                Ok(game) => games.push(game),
                Err(error) => problems.push(format!("{folder}: {error}")),
            }
        }
        games.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        problems.sort();
        Ok((games, problems))
    }

    fn game(&self, id: Uuid) -> Result<Game> {
        let folder = id.to_string();
        read_manifest(&self.root.join("games").join(&folder), &folder)
            .map_err(|_| invalid("game ID was not found in the library").into())
    }

    fn game_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("games").join(id.to_string())
    }

    fn profile_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("profiles").join(id.to_string())
    }

    fn retained_marker(&self, id: Uuid) -> PathBuf {
        self.root
            .join("profiles")
            .join(format!("{id}.retained.json"))
    }

    fn entry_file(&self, id: Uuid) -> Result<PathBuf> {
        let content = self.game_dir(id).join("content");
        let metadata = fs::symlink_metadata(&content)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid("game content directory is invalid").into());
        }
        let entry = content.join("index.html");
        let metadata = fs::symlink_metadata(&entry)
            .map_err(|_| invalid("game files are missing; add the original file again"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("game entry point is not a regular file").into());
        }
        Ok(entry)
    }

    /// Only generated game IDs can select content and persistent browser storage.
    pub fn player_paths(&self, id: Uuid) -> Result<(PathBuf, PathBuf)> {
        self.game(id)?;
        self.entry_file(id)?;
        let content = self.game_dir(id).join("content");
        let profiles = self.root.join("profiles");
        fs::create_dir_all(&profiles)?;
        if !fs::symlink_metadata(&profiles)?.file_type().is_dir() {
            return Err(invalid("browser profile directory is invalid").into());
        }
        let profile = self.profile_dir(id);
        fs::create_dir_all(&profile)?;
        if !fs::symlink_metadata(&profile)?.file_type().is_dir() {
            return Err(invalid("game browser profile is invalid").into());
        }
        Ok((fs::canonicalize(content)?, fs::canonicalize(profile)?))
    }

    /// Prevents two players, or a player and a removal, from using one profile at once.
    pub fn lock_game(&self, id: Uuid) -> Result<GameLock> {
        let profiles = self.root.join("profiles");
        fs::create_dir_all(&profiles)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(profiles.join(format!("{id}.lock")))?;
        match file.try_lock() {
            Ok(()) => Ok(GameLock { _file: file }),
            Err(fs::TryLockError::WouldBlock) => {
                Err(invalid("this game is already open in a player window").into())
            }
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    pub fn record_play(&self, id: Uuid) -> Result<()> {
        self.game(id)?;
        let mut activity = self.read_activity(id);
        activity.last_played = Some(now_unix());
        activity.play_count = activity.play_count.saturating_add(1);
        write_json_atomic(&self.game_dir(id).join("activity.json"), &activity)
    }

    fn read_activity(&self, id: Uuid) -> Activity {
        let path = self.game_dir(id).join("activity.json");
        fs::symlink_metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() <= MAX_MANIFEST_BYTES)
            .and_then(|_| File::open(&path).ok())
            .and_then(|file| serde_json::from_reader(file).ok())
            .unwrap_or_default()
    }

    pub fn rename(&self, id: Uuid, title: &str) -> Result<Game> {
        let mut game = self.game(id)?;
        game.title =
            clean_title(title).ok_or_else(|| invalid("enter a title with 1 to 80 characters"))?;
        write_json_atomic(&self.game_dir(id).join("game.json"), &game)?;
        Ok(game)
    }

    /// Deletes this game's browser profile: localStorage, IndexedDB, cookies, and caches.
    pub fn reset_saves(&self, id: Uuid) -> Result<()> {
        self.game(id)?;
        let lock = self.lock_game(id)?;
        let profile = self.profile_dir(id);
        if fs::symlink_metadata(&profile).is_ok_and(|metadata| metadata.is_dir()) {
            remove_profile(&profile)?;
        }
        drop(lock);
        Ok(())
    }

    /// Removes a game's files. With `keep_saves`, its profile stays and re-adding the exact
    /// same file later restores it; otherwise the profile is deleted too.
    pub fn remove(&self, id: Uuid, keep_saves: bool) -> Result<()> {
        let game = self.game(id)?;
        let lock = self.lock_game(id)?;
        let profile = self.profile_dir(id);
        let has_profile = fs::symlink_metadata(&profile).is_ok_and(|metadata| metadata.is_dir());
        let marker = self.retained_marker(id);
        if keep_saves && has_profile {
            let sha = game.source_sha256.as_ref().ok_or_else(|| {
                invalid("re-add the original game file before removing this legacy game with saves")
            })?;
            if fs::symlink_metadata(&marker).is_ok() {
                return Err(invalid("a retained-saves marker already exists for this game").into());
            }
            // Do every fallible preparation before hiding the game from the library.
            write_json_atomic(
                &marker,
                &RetainedSaves {
                    source_kind: game.source_kind.clone(),
                    source_sha256: sha.clone(),
                    title: game.title.clone(),
                },
            )?;
        }
        // Move the folder out of `games/` first so a partial delete is never listed.
        let trash = self
            .root
            .join("incoming")
            .join(format!("removed-{id}-{}", Uuid::new_v4().simple()));
        if let Err(error) = fs::rename(self.game_dir(id), &trash) {
            if keep_saves && has_profile {
                let _ = fs::remove_file(&marker);
            }
            return Err(error.into());
        }
        if has_profile && !keep_saves {
            remove_profile(&profile)?;
        }
        drop(lock);
        if !keep_saves {
            let _ = fs::remove_file(self.root.join("profiles").join(format!("{id}.lock")));
        }
        fs::remove_dir_all(trash)?;
        Ok(())
    }

    fn find_retained(&self, kind: &SourceKind, sha: &str) -> Option<Uuid> {
        let items = fs::read_dir(self.root.join("profiles")).ok()?;
        items.filter_map(|item| item.ok()).find_map(|item| {
            let name = item.file_name().into_string().ok()?;
            let id = Uuid::parse_str(name.strip_suffix(".retained.json")?).ok()?;
            let metadata = fs::symlink_metadata(item.path()).ok()?;
            if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
                return None;
            }
            let retained: RetainedSaves =
                serde_json::from_reader(File::open(item.path()).ok()?).ok()?;
            (retained.source_kind == *kind
                && retained.source_sha256 == sha
                && !self.game_dir(id).exists()
                && self.profile_dir(id).is_dir())
            .then_some(id)
        })
    }

    /// True for a library marked as synthetic test data with a `SYNTHETIC-TEST-LIBRARY` file in its
    /// root, as the CI capture scripts do. The launcher then shows a "Synthetic test library" badge.
    pub fn is_synthetic_test_library(&self) -> bool {
        regular_file(&self.root.join("SYNTHETIC-TEST-LIBRARY"))
    }

    /// Sets or clears the player's own note of where the game came from.
    pub fn set_source(&self, id: Uuid, source: Option<&str>) -> Result<Game> {
        let mut game = self.game(id)?;
        game.source = match source {
            None => None,
            Some(raw) if raw.trim().is_empty() => None,
            Some(raw) => Some(
                clean_source(raw)
                    .ok_or_else(|| invalid("enter a source with 1 to 32 characters"))?,
            ),
        };
        write_json_atomic(&self.game_dir(id).join("game.json"), &game)?;
        Ok(game)
    }

    pub fn set_favorite(&self, id: Uuid, favorite: bool) -> Result<Game> {
        let mut game = self.game(id)?;
        game.favorite = favorite;
        write_json_atomic(&self.game_dir(id).join("game.json"), &game)?;
        Ok(game)
    }

    /// Replaces the game's cover with an image file the player picked. The image is untrusted:
    /// it is decoded by memory-safe Rust code with size limits, scaled down, and saved as a new
    /// PNG inside the library. The previous custom cover is kept so the change can be undone.
    pub fn set_cover(&self, id: Uuid, image: impl AsRef<Path>) -> Result<()> {
        self.game(id)?;
        let png = cover_png_from_file(image.as_ref())?;
        let folder = self.game_dir(id);
        let staged = folder.join(format!("cover-{}.tmp", Uuid::new_v4().simple()));
        let result = (|| -> Result<()> {
            let mut file = new_file(&staged)?;
            file.write_all(&png)?;
            file.sync_all()?;
            drop(file);
            let current = folder.join(CUSTOM_COVER);
            if regular_file(&current) {
                fs::rename(&current, folder.join(PREVIOUS_COVER))?;
            }
            fs::rename(&staged, &current)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result
    }

    /// Brings back the previous custom cover. Undoing twice returns to where it started.
    pub fn undo_cover(&self, id: Uuid) -> Result<()> {
        self.game(id)?;
        let folder = self.game_dir(id);
        let current = folder.join(CUSTOM_COVER);
        let previous = folder.join(PREVIOUS_COVER);
        if !regular_file(&previous) {
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

    /// Goes back to the automatic cover. The custom one is kept, so Undo can restore it.
    pub fn reset_cover(&self, id: Uuid) -> Result<()> {
        self.game(id)?;
        let folder = self.game_dir(id);
        let current = folder.join(CUSTOM_COVER);
        if regular_file(&current) {
            fs::rename(&current, folder.join(PREVIOUS_COVER))?;
        }
        Ok(())
    }

    /// Whether this ID is an HTML game in the library (store games live in [`stores`]).
    pub fn has_game(&self, id: Uuid) -> bool {
        self.game(id).is_ok()
    }

    /// The cover the launcher serves for any game ID: an HTML game's cover, or a store game's.
    pub fn display_cover_file(&self, id: Uuid) -> Option<PathBuf> {
        self.cover_file(id)
            .or_else(|| stores::cover_file_at(&self.root, id))
    }

    /// The PNG the launcher shows for this game: the custom cover, else the automatic one.
    pub fn cover_file(&self, id: Uuid) -> Option<PathBuf> {
        let folder = self.game_dir(id);
        [CUSTOM_COVER, AUTO_COVER]
            .into_iter()
            .map(|name| folder.join(name))
            .find(|path| regular_file(path))
    }

    fn cover_kind(&self, id: Uuid) -> Option<&'static str> {
        let folder = self.game_dir(id);
        if regular_file(&folder.join(CUSTOM_COVER)) {
            Some("custom")
        } else if regular_file(&folder.join(AUTO_COVER)) {
            Some("auto")
        } else {
            None
        }
    }

    fn cover_version(&self, id: Uuid) -> u64 {
        self.cover_file(id)
            .and_then(|path| fs::symlink_metadata(path).ok())
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |time| time.as_millis() as u64)
    }

    /// Looks once for a cover image among the game's own top-level files (an icon, thumbnail,
    /// or screenshot with a well-known name) and stores a scaled PNG copy of it. A marker
    /// records that the search ran, so it is not repeated for the same game.
    pub fn ensure_auto_cover(&self, id: Uuid) -> Result<()> {
        self.game(id)?;
        let folder = self.game_dir(id);
        let auto = folder.join(AUTO_COVER);
        let searched = folder.join(AUTO_COVER_SEARCHED);
        if regular_file(&auto) || fs::symlink_metadata(&searched).is_ok() {
            return Ok(());
        }
        let mut files: Vec<(usize, PathBuf)> = Vec::new();
        for item in fs::read_dir(folder.join("content"))?.flatten() {
            let name = item.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(rank) = AUTO_COVER_NAMES.iter().position(|known| *known == name) else {
                continue;
            };
            if regular_file(&item.path()) {
                files.push((rank, item.path()));
            }
        }
        files.sort();
        let found = files
            .into_iter()
            .find_map(|(_, path)| cover_png_from_file(&path).ok());
        if let Some(png) = found {
            let staged = folder.join(format!("cover-{}.tmp", Uuid::new_v4().simple()));
            let mut file = new_file(&staged)?;
            file.write_all(&png)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&staged, &auto)?;
        }
        File::create(searched)?;
        Ok(())
    }
}

/// A regular file, not a link or folder.
fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Custom cover chosen by the player, its predecessor (for Undo), and the automatic cover.
const CUSTOM_COVER: &str = "cover.png";
const PREVIOUS_COVER: &str = "cover-previous.png";
const AUTO_COVER: &str = "auto-cover.png";
const AUTO_COVER_SEARCHED: &str = "auto-cover.searched";
/// Well-known names for artwork inside a game, best first.
const AUTO_COVER_NAMES: [&str; 16] = [
    "cover.png",
    "cover.jpg",
    "cover.jpeg",
    "cover.webp",
    "thumbnail.png",
    "thumbnail.jpg",
    "thumb.png",
    "screenshot.png",
    "screenshot.jpg",
    "splash.png",
    "icon-512.png",
    "icon.png",
    "logo.png",
    "apple-touch-icon.png",
    "favicon.png",
    "icon.webp",
];
const MAX_COVER_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_COVER_SOURCE_SIDE: u32 = 8192;
/// Covers are stored at most this size; the launcher's tiles are portrait.
pub const COVER_WIDTH: u32 = 600;
pub const COVER_HEIGHT: u32 = 900;
/// Largest stored cover the launcher will serve (a 600 × 900 RGBA PNG is far smaller).
pub const MAX_STORED_COVER_BYTES: u64 = 8 * 1024 * 1024;

/// Decodes an untrusted cover image file; see [`cover_png_from_bytes`].
pub fn cover_png_from_file(path: &Path) -> Result<Vec<u8>> {
    encode_cover(decode_cover_file(path)?)
}

/// Decodes an untrusted PNG, JPEG, GIF, or WebP with memory-safe Rust decoders under strict
/// limits (16 MiB file, 8192 px per side, 256 MiB of decoder memory), scales it to fit
/// 600 × 900, and returns a freshly encoded PNG. Only the pixels survive.
pub fn cover_png_from_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    encode_cover(decode_cover(bytes)?)
}

/// Reads and decodes an untrusted image file (a regular file of at most 16 MiB) under the limits
/// of [`cover_png_from_bytes`], without scaling it.
pub(crate) fn decode_cover_file(path: &Path) -> Result<image::DynamicImage> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(invalid("the cover must be a regular image file").into());
    }
    if metadata.len() == 0 || metadata.len() > MAX_COVER_SOURCE_BYTES {
        return Err(invalid("the cover image is empty or larger than 16 MiB").into());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_COVER_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    decode_cover(&bytes)
}

/// Scales a decoded cover to fit 600 × 900 and encodes it as a fresh RGBA PNG.
pub(crate) fn encode_cover(image: image::DynamicImage) -> Result<Vec<u8>> {
    use image::{ImageFormat, imageops::FilterType};
    let image = if image.width() > COVER_WIDTH || image.height() > COVER_HEIGHT {
        image.resize(COVER_WIDTH, COVER_HEIGHT, FilterType::Triangle)
    } else {
        image
    };
    let image = image::DynamicImage::ImageRgba8(image.to_rgba8());
    let mut png = Vec::new();
    image
        .write_to(&mut io::Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|_| invalid("the cover could not be saved"))?;
    Ok(png)
}

/// Decodes untrusted image bytes under the limits of [`cover_png_from_bytes`], without scaling.
fn decode_cover(bytes: &[u8]) -> Result<image::DynamicImage> {
    use image::{ImageFormat, ImageReader, Limits};
    if bytes.len() as u64 > MAX_COVER_SOURCE_BYTES {
        return Err(invalid("the cover image is larger than 16 MiB").into());
    }
    let mut reader = ImageReader::new(io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid("the cover could not be read"))?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP)
    ) {
        return Err(invalid("covers must be PNG, JPEG, GIF, or WebP images").into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_COVER_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_COVER_SOURCE_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| invalid("the cover image is damaged or too large").into())
}

/// Trims, drops control characters, and limits sources to 32 characters.
fn clean_source(raw: &str) -> Option<String> {
    let source: String = raw.chars().filter(|c| !c.is_control()).collect();
    let source = source.trim();
    (!source.is_empty() && source.chars().count() <= 32).then(|| source.to_owned())
}

/// The operating systems GameStrike knows where to keep a library on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    MacOs,
    /// Linux and other freedesktop-style systems.
    Unix,
}

impl HostOs {
    pub fn current() -> Self {
        if cfg!(windows) {
            HostOs::Windows
        } else if cfg!(target_os = "macos") {
            HostOs::MacOs
        } else {
            HostOs::Unix
        }
    }
}

/// Where the launcher keeps its library when `--library` is not given.
///
/// - Windows: `%LOCALAPPDATA%\GameStrike\Library` (unchanged, so existing libraries and saves keep working)
/// - macOS: `~/Library/Application Support/GameStrike/Library`
/// - Linux: `$XDG_DATA_HOME/gamestrike/library`, or `~/.local/share/gamestrike/library`
pub fn default_library_root() -> Result<PathBuf> {
    default_library_root_for(HostOs::current(), |name| std::env::var_os(name))
}

/// [`default_library_root`] for a given OS and environment, so every branch can be tested anywhere.
pub fn default_library_root_for(
    os: HostOs,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    // Relative values would make the library depend on the current directory, so they are ignored.
    let absolute = |name: &str| {
        env(name)
            .map(PathBuf::from)
            .filter(|path| is_absolute_on(os, path))
    };
    match os {
        HostOs::Windows => Ok(absolute("LOCALAPPDATA")
            .ok_or_else(|| invalid("LOCALAPPDATA is unavailable"))?
            .join("GameStrike")
            .join("Library")),
        HostOs::MacOs => Ok(absolute("HOME")
            .ok_or_else(|| invalid("HOME is unavailable"))?
            .join("Library")
            .join("Application Support")
            .join("GameStrike")
            .join("Library")),
        HostOs::Unix => {
            let data = match absolute("XDG_DATA_HOME") {
                Some(data) => data,
                None => absolute("HOME")
                    .ok_or_else(|| invalid("neither XDG_DATA_HOME nor HOME is set"))?
                    .join(".local")
                    .join("share"),
            };
            Ok(data.join("gamestrike").join("library"))
        }
    }
}

/// `Path::is_absolute` follows the build OS; this follows `os`, so each branch is testable anywhere.
fn is_absolute_on(os: HostOs, path: &Path) -> bool {
    let text = path.to_string_lossy();
    let bytes = text.as_bytes();
    match os {
        HostOs::Windows => {
            (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && (bytes[2] == b'\\' || bytes[2] == b'/'))
                || text.starts_with(r"\\")
        }
        HostOs::MacOs | HostOs::Unix => text.starts_with('/'),
    }
}

/// The `gamestrike-player` executable that sits beside the running program.
pub fn sibling_player() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?
        .with_file_name(format!("gamestrike-player{}", std::env::consts::EXE_SUFFIX)))
}

/// Deletes browser data the engine keeps outside the library folder.
///
/// On Windows and Linux every save lives in `LIBRARY/profiles/<id>`, so this does nothing. On macOS,
/// WKWebView keeps each game's data store under an identifier equal to the game ID, which only
/// WebKit can delete; the sibling player does that with `--clear-engine-data`. Call this before
/// [`Library::reset_saves`] or a removal without kept saves.
pub fn clear_engine_data(root: &Path, id: Uuid) -> Result<()> {
    if HostOs::current() != HostOs::MacOs {
        return Ok(());
    }
    let player = sibling_player()?;
    if !player.is_file() {
        return Err(invalid(
            "gamestrike-player is needed beside this program to clear saves on macOS",
        )
        .into());
    }
    let output = std::process::Command::new(player)
        .arg("--library")
        .arg(root)
        .arg("--id")
        .arg(id.to_string())
        .arg("--clear-engine-data")
        .stdin(std::process::Stdio::null())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.lines().last().unwrap_or("").trim();
        Err(io::Error::other(format!(
            "could not clear the game's WebKit data ({}){}{}",
            output.status,
            if detail.is_empty() { "" } else { ": " },
            detail.chars().take(200).collect::<String>()
        ))
        .into())
    }
}

fn read_manifest(folder: &Path, folder_name: &str) -> Result<Game> {
    let path = folder.join("game.json");
    let metadata = fs::symlink_metadata(&path).map_err(|_| invalid("game.json is missing"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err(invalid("invalid game manifest").into());
    }
    let game: Game = serde_json::from_reader(File::open(path)?)
        .map_err(|_| invalid("game.json is not a valid manifest"))?;
    if folder_name != game.id.to_string() {
        return Err(invalid("game directory and manifest ID differ").into());
    }
    if clean_title(&game.title).is_none() {
        return Err(invalid("game title is invalid").into());
    }
    Ok(game)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid("metadata is too large").into());
    }
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let mut file = new_file(&temporary)?;
        file.write_all(&bytes)?;
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

/// Deletes a game's browser profile. On Windows the WebView2 runtime's helper processes can keep
/// files in the profile open for a moment after the player exits, so a refused delete is retried
/// for up to five seconds before the error is reported.
fn remove_profile(profile: &Path) -> io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match fs::remove_dir_all(profile) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if profile_busy(&error) && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Errors that mean another process still has a file in the profile open. On Windows these include
/// ERROR_SHARING_VIOLATION (32) and ERROR_LOCK_VIOLATION (33), which Rust reports as uncategorized.
fn profile_busy(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::DirectoryNotEmpty
    ) || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
}

fn directory_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let Ok(items) = fs::read_dir(directory) else {
            continue;
        };
        for item in items.flatten() {
            seen += 1;
            if seen > MAX_PROFILE_WALK {
                return total;
            }
            let Ok(metadata) = fs::symlink_metadata(item.path()) else {
                continue;
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                directories.push(item.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Trims, drops control characters, and limits titles to 80 characters.
fn clean_title(raw: &str) -> Option<String> {
    let title: String = raw.chars().filter(|c| !c.is_control()).collect();
    let title = title.trim();
    (!title.is_empty() && title.chars().count() <= MAX_TITLE_CHARS).then(|| title.to_owned())
}

fn hash_source(source: &Path, expected_len: u64) -> Result<String> {
    let mut file = File::open(source)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| invalid("source size overflow"))?;
        if size > MAX_SOURCE_BYTES {
            return Err(invalid("source exceeds 256 MiB").into());
        }
        digest.update(&buffer[..count]);
    }
    if size != expected_len {
        return Err(invalid("source changed while importing").into());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn hash_content_tree(root: &Path) -> Result<String> {
    let root_metadata = fs::symlink_metadata(root)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(invalid("game content directory is invalid").into());
    }
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut total = 0u64;
    while let Some(directory) = directories.pop() {
        for item in fs::read_dir(directory)? {
            let path = item?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(invalid("game content contains a link").into());
            }
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| invalid("game content size overflow"))?;
                if metadata.len() > MAX_FILE_BYTES
                    || total > MAX_TOTAL_BYTES
                    || files.len() >= MAX_FILES
                {
                    return Err(invalid("game content exceeds import limits").into());
                }
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or_else(|| invalid("game content path is not UTF-8"))?
                    .replace('\\', "/");
                files.push((relative, path, metadata.len()));
            } else {
                return Err(invalid("game content contains a special file").into());
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 32 * 1024];
    for (relative, path, length) in files {
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative.as_bytes());
        digest.update(length.to_le_bytes());
        let mut file = File::open(path)?;
        let mut read_total = 0u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read_total = read_total
                .checked_add(count as u64)
                .ok_or_else(|| invalid("game content size overflow"))?;
            if read_total > length {
                return Err(invalid("game content changed during import").into());
            }
            digest.update(&buffer[..count]);
        }
        if read_total != length {
            return Err(invalid("game content changed during import").into());
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn import_zip(source: &Path, content: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(source)?)
        .map_err(|_| invalid("the file is not a valid ZIP archive"))?;
    if archive.is_empty() || archive.len() > MAX_FILES {
        return Err(invalid("ZIP has no entries or too many entries").into());
    }
    let mut entries = Vec::with_capacity(archive.len());
    let mut seen = HashSet::new();
    let mut regular_files = HashSet::new();
    let mut entry_points = Vec::new();
    let mut total = 0;

    // Decompress every member to a sink before creating extracted files. This checks
    // actual expansion and CRC, not only the sizes claimed in the ZIP directory.
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        if file.name_raw().contains(&0) || file.enclosed_name().is_none() {
            return Err(invalid("ZIP contains an unsafe path").into());
        }
        let relative = safe_content_path(file.name())?;
        let is_dir = file.is_dir();
        validate_mode(file.unix_mode(), is_dir)?;
        let key = relative.to_string_lossy().to_lowercase();
        if !seen.insert(key.clone()) {
            return Err(invalid("ZIP contains duplicate paths").into());
        }
        if !is_dir {
            regular_files.insert(key);
            if file.compression() != CompressionMethod::Stored
                && file.compression() != CompressionMethod::Deflated
            {
                return Err(invalid("ZIP compression method is unsupported").into());
            }
            if file.size() > MAX_FILE_BYTES
                || file.size()
                    > file
                        .compressed_size()
                        .saturating_mul(MAX_EXPANSION_RATIO)
                        .max(4096)
            {
                return Err(invalid("ZIP member expands excessively").into());
            }
            copy_bounded(&mut file, &mut io::sink(), &mut total, MAX_FILE_BYTES)?;
            if relative.components().count() <= 2
                && relative.file_name().is_some_and(|name| {
                    name.to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("index.html"))
                })
            {
                entry_points.push(relative.clone());
            }
        }
        entries.push((relative, is_dir));
    }
    let entry = match entry_points.as_slice() {
        [entry] if entry.file_name().is_some_and(|name| name == "index.html") => entry,
        [] => return Err(invalid("ZIP needs a root or single-wrapper index.html").into()),
        _ => return Err(invalid("ZIP has ambiguous entry points").into()),
    };
    let wrapper = entry.parent().filter(|path| !path.as_os_str().is_empty());
    if let Some(wrapper) = wrapper
        && entries.iter().any(|(relative, is_dir)| {
            !relative.starts_with(wrapper) || (relative == wrapper && !*is_dir)
        })
    {
        return Err(invalid("ZIP has files outside its entry-point wrapper").into());
    }
    for (relative, _) in &entries {
        let mut parent = relative.parent();
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            if regular_files.contains(&path.to_string_lossy().to_lowercase()) {
                return Err(invalid("ZIP uses a file as a directory").into());
            }
            parent = path.parent();
        }
    }

    // The same archive handle is reused. The extraction pass enforces byte limits again.
    total = 0;
    for (index, (relative, is_dir)) in entries.iter().enumerate() {
        let extracted = if let Some(wrapper) = wrapper {
            relative.strip_prefix(wrapper)?
        } else {
            relative.as_path()
        };
        if extracted.as_os_str().is_empty() {
            continue;
        }
        let path = content.join(extracted);
        if *is_dir {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut input = archive.by_index(index)?;
        if safe_content_path(input.name())? != *relative || input.is_dir() {
            return Err(invalid("ZIP changed while importing").into());
        }
        let mut output = new_file(&path)?;
        copy_bounded(&mut input, &mut output, &mut total, MAX_FILE_BYTES)?;
    }
    Ok(())
}

fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    total: &mut u64,
    file_limit: u64,
) -> Result<()> {
    let mut file_bytes = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        file_bytes = file_bytes
            .checked_add(count as u64)
            .ok_or_else(|| invalid("file size overflow"))?;
        *total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("total size overflow"))?;
        if file_bytes > file_limit || *total > MAX_TOTAL_BYTES {
            return Err(invalid("import exceeds expanded size limit").into());
        }
        writer.write_all(&buffer[..count])?;
    }
}

pub fn safe_content_path(name: &str) -> Result<PathBuf> {
    let name = name.strip_suffix('/').unwrap_or(name);
    if name.is_empty()
        || name.len() > MAX_PATH_BYTES
        || name.starts_with('/')
        || name.contains('\\')
    {
        return Err(invalid("ZIP contains an unsafe path").into());
    }
    let mut output = PathBuf::new();
    for part in name.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(' ')
            || part.ends_with('.')
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        {
            return Err(invalid("ZIP contains an unsafe path component").into());
        }
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit()
                && base.as_bytes()[3] != b'0')
        {
            return Err(invalid("ZIP contains a Windows device name").into());
        }
        output.push(part);
    }
    Ok(output)
}

fn validate_mode(mode: Option<u32>, is_dir: bool) -> Result<()> {
    let kind = mode.unwrap_or(0) & 0o170000;
    if kind != 0 && kind != if is_dir { 0o040000 } else { 0o100000 } {
        return Err(invalid("ZIP contains a link or special file").into());
    }
    Ok(())
}

fn display_title(source: &Path) -> String {
    let title = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Imported game");
    let title: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect();
    clean_title(&title).unwrap_or_else(|| "Imported game".to_owned())
}

fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn archive(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        for (name, body) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, SimpleFileOptions::default())
                    .unwrap();
            } else {
                zip.start_file(*name, SimpleFileOptions::default()).unwrap();
                zip.write_all(body).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn html_import_is_listed_and_copied() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Tiny Game.html");
        fs::write(&source, b"<!doctype html><title>Tiny</title>").unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let game = library.import(&source).unwrap();
        assert_eq!(library.list().unwrap(), vec![game.clone()]);
        assert_eq!(
            fs::read(
                dir.path()
                    .join("library/games")
                    .join(game.id.to_string())
                    .join("content/index.html")
            )
            .unwrap(),
            fs::read(source).unwrap()
        );
    }

    #[test]
    fn exact_reimport_reuses_game_and_preserves_its_profile() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Puzzle.html");
        fs::write(&source, b"<!doctype html><title>Puzzle</title>").unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let first = library.import_with_result(&source).unwrap();
        assert!(!first.already_present);
        let (_, profile) = library.player_paths(first.game.id).unwrap();
        fs::write(profile.join("save-marker"), b"keep this save").unwrap();
        let second = library.import_with_result(&source).unwrap();
        assert!(second.already_present);
        assert_eq!(second.game.id, first.game.id);
        assert_eq!(library.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"keep this save"
        );
        fs::write(&source, b"<!doctype html><title>Another game</title>").unwrap();
        assert!(!library.import_with_result(&source).unwrap().already_present);
        assert_eq!(library.list().unwrap().len(), 2);
    }

    #[test]
    fn manifests_without_fingerprints_remain_readable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Legacy.html");
        fs::write(&source, b"<html>legacy</html>").unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let game = library.import(&source).unwrap();
        let manifest = dir
            .path()
            .join("library/games")
            .join(game.id.to_string())
            .join("game.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("source_sha256");
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(library.list().unwrap()[0].source_sha256, None);
    }

    #[test]
    fn removing_legacy_game_requires_safe_fingerprint_migration() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Legacy.html");
        fs::write(&source, b"<html>legacy game</html>").unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let first = library.import(&source).unwrap();
        let manifest = root
            .join("games")
            .join(first.id.to_string())
            .join("game.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("source_sha256");
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let (_, profile) = library.player_paths(first.id).unwrap();
        fs::write(profile.join("save-marker"), b"legacy progress").unwrap();

        assert!(library.remove(first.id, true).is_err());
        assert_eq!(library.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"legacy progress"
        );

        let migrated = library.import_with_result(&source).unwrap();
        assert!(migrated.already_present);
        assert_eq!(migrated.game.id, first.id);
        assert!(migrated.game.source_sha256.is_some());
        library.remove(first.id, true).unwrap();
        let second = library.import_with_result(&source).unwrap();
        assert!(second.restored_saves);
        assert_eq!(second.game.id, first.id);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"legacy progress"
        );
    }

    #[test]
    fn failed_retained_marker_write_keeps_game_in_library() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Game.html");
        fs::write(&source, b"<html>game</html>").unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = library.import(&source).unwrap();
        let (_, profile) = library.player_paths(game.id).unwrap();
        fs::write(profile.join("save-marker"), b"progress").unwrap();
        fs::create_dir(
            root.join("profiles")
                .join(format!("{}.retained.json", game.id)),
        )
        .unwrap();

        assert!(library.remove(game.id, true).is_err());
        assert_eq!(library.list().unwrap().len(), 1);
        assert!(library.entry_file(game.id).is_ok());
        assert_eq!(fs::read(profile.join("save-marker")).unwrap(), b"progress");
    }

    #[test]
    fn failed_game_move_cleans_new_retained_marker() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Game.html");
        fs::write(&source, b"<html>game</html>").unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = library.import(&source).unwrap();
        let (_, profile) = library.player_paths(game.id).unwrap();
        fs::write(profile.join("save-marker"), b"progress").unwrap();
        fs::remove_dir(root.join("incoming")).unwrap();
        fs::write(root.join("incoming"), b"blocked").unwrap();

        assert!(library.remove(game.id, true).is_err());
        assert_eq!(library.list().unwrap().len(), 1);
        assert!(
            !root
                .join("profiles")
                .join(format!("{}.retained.json", game.id))
                .exists()
        );
        assert_eq!(fs::read(profile.join("save-marker")).unwrap(), b"progress");
    }

    #[test]
    fn reimporting_legacy_html_keeps_its_game_id_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Legacy.html");
        fs::write(&source, b"<html>legacy game</html>").unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let first = library.import(&source).unwrap();
        let manifest = root
            .join("games")
            .join(first.id.to_string())
            .join("game.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("source_sha256");
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let (_, profile) = library.player_paths(first.id).unwrap();
        fs::write(profile.join("save-marker"), b"legacy progress").unwrap();

        let second = library.import_with_result(&source).unwrap();
        assert!(second.already_present);
        assert_eq!(second.game.id, first.id);
        assert_eq!(library.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"legacy progress"
        );
    }

    #[test]
    fn reimporting_legacy_zip_keeps_its_game_id_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Legacy.zip");
        archive(
            &source,
            &[
                ("index.html", b"<html>legacy zip</html>"),
                ("game.js", b"//local"),
            ],
        );
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let first = library.import(&source).unwrap();
        let manifest = root
            .join("games")
            .join(first.id.to_string())
            .join("game.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("source_sha256");
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let (_, profile) = library.player_paths(first.id).unwrap();
        fs::write(profile.join("save-marker"), b"zip progress").unwrap();

        let second = library.import_with_result(&source).unwrap();
        assert!(second.already_present);
        assert_eq!(second.game.id, first.id);
        assert_eq!(library.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"zip progress"
        );
    }

    #[test]
    fn exact_zip_reimport_does_not_extract_another_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Puzzle.zip");
        archive(&source, &[("index.html", b"<html>puzzle</html>")]);
        let library = Library::open(dir.path().join("library")).unwrap();
        let first = library.import_with_result(&source).unwrap();
        let (_, profile) = library.player_paths(first.game.id).unwrap();
        fs::write(profile.join("save-marker"), b"zip progress").unwrap();
        let second = library.import_with_result(&source).unwrap();
        assert!(second.already_present);
        assert_eq!(first.game.id, second.game.id);
        assert_eq!(library.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"zip progress"
        );
    }

    #[test]
    fn interrupted_staging_is_not_published_or_removed_by_the_next_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let stale = root.join("incoming/interrupted/content/index.html");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, b"partial import").unwrap();
        let source = dir.path().join("Good.html");
        fs::write(&source, b"<html>good</html>").unwrap();
        let game = library.import(&source).unwrap();
        assert_eq!(library.list().unwrap(), vec![game]);
        assert_eq!(fs::read(stale).unwrap(), b"partial import");
    }

    #[test]
    fn corrupt_zip_is_rejected_without_affecting_an_existing_save() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let good = dir.path().join("Good.html");
        fs::write(&good, b"<html>good</html>").unwrap();
        let game = library.import(&good).unwrap();
        let (_, profile) = library.player_paths(game.id).unwrap();
        fs::write(profile.join("save-marker"), b"keep progress").unwrap();

        let source = dir.path().join("Corrupt.zip");
        let marker = b"unique-zip-content-marker";
        let file = File::create(&source).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file(
            "index.html",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(marker).unwrap();
        zip.finish().unwrap();
        let mut bytes = fs::read(&source).unwrap();
        let at = bytes
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap();
        bytes[at] ^= 1;
        fs::write(&source, bytes).unwrap();

        assert!(library.import(&source).is_err());
        assert_eq!(library.list().unwrap(), vec![game]);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"keep progress"
        );
        assert_eq!(
            fs::read_dir(dir.path().join("library/incoming"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn player_profiles_are_stable_and_separate() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let one = dir.path().join("one.html");
        let two = dir.path().join("two.html");
        fs::write(&one, b"<html>one</html>").unwrap();
        fs::write(&two, b"<html>two</html>").unwrap();
        let one = library.import(one).unwrap();
        let two = library.import(two).unwrap();
        let (one_content, one_profile) = library.player_paths(one.id).unwrap();
        let (two_content, two_profile) = library.player_paths(two.id).unwrap();
        assert_ne!(one_content, two_content);
        assert_ne!(one_profile, two_profile);
        assert_eq!(one_profile, library.player_paths(one.id).unwrap().1);
    }

    #[test]
    fn zip_import_rejects_traversal_and_missing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("game.zip");
        archive(
            &source,
            &[("index.html", b"<html></html>"), ("../escape.txt", b"bad")],
        );
        let library = Library::open(dir.path().join("library")).unwrap();
        assert!(library.import(&source).is_err());
        assert!(library.list().unwrap().is_empty());
        assert!(!dir.path().join("escape.txt").exists());
        archive(
            &dir.path().join("missing.zip"),
            &[("other.html", b"<html></html>")],
        );
        assert!(library.import(dir.path().join("missing.zip")).is_err());
    }

    #[test]
    fn zip_import_rejects_duplicate_names() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("game.zip");
        archive(
            &source,
            &[
                ("index.html", b"<html></html>"),
                ("INDEX.HTML", b"different"),
            ],
        );
        let library = Library::open(dir.path().join("library")).unwrap();
        assert!(library.import(source).is_err());
        assert!(library.list().unwrap().is_empty());
    }

    #[test]
    fn zip_import_rejects_file_directory_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("conflict.zip");
        archive(
            &source,
            &[
                ("index.html", b"<html></html>"),
                ("assets", b"ordinary file"),
                ("assets/child.js", b"console.log('fixture')"),
            ],
        );
        let library = Library::open(dir.path().join("library")).unwrap();
        assert!(library.import(source).is_err());
        assert!(library.list().unwrap().is_empty());
    }

    #[test]
    fn zip_import_rejects_excessive_compression_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("expansion.zip");
        let file = File::create(&source).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file(
            "index.html",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(b"<html></html>").unwrap();
        zip.start_file(
            "large.bin",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&vec![0u8; 1024 * 1024]).unwrap();
        zip.finish().unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        assert!(library.import(source).is_err());
        assert!(library.list().unwrap().is_empty());
    }

    #[test]
    fn zip_import_copies_assets_without_executing_them() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("game.zip");
        archive(
            &source,
            &[
                ("index.html", b"<script src=\"code.js\"></script>"),
                ("code.js", b"localStorage.setItem('score','7')"),
            ],
        );
        let library = Library::open(dir.path().join("library")).unwrap();
        let game = library.import(&source).unwrap();
        let content = dir
            .path()
            .join("library/games")
            .join(game.id.to_string())
            .join("content");
        assert_eq!(
            fs::read(content.join("code.js")).unwrap(),
            b"localStorage.setItem('score','7')"
        );
    }

    #[test]
    fn wrapped_zip_import_keeps_relative_assets_and_reuses_profile() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("wrapped.zip");
        archive(
            &source,
            &[
                ("Arcade/", b""),
                (
                    "Arcade/index.html",
                    b"<link rel=\"stylesheet\" href=\"assets/game.css\"><script src=\"assets/game.js\"></script>",
                ),
                ("Arcade/assets/", b""),
                ("Arcade/assets/game.css", b"body { color: orange }"),
                ("Arcade/assets/game.js", b"window.gameLoaded = true"),
            ],
        );
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let first = library.import_with_result(&source).unwrap();
        assert!(!first.already_present);
        assert_eq!(first.game.entry, "index.html");
        let (content, profile) = library.player_paths(first.game.id).unwrap();
        assert!(
            fs::read_to_string(content.join("index.html"))
                .unwrap()
                .contains("assets/game.js")
        );
        assert_eq!(
            fs::read(content.join("assets/game.css")).unwrap(),
            b"body { color: orange }"
        );
        assert_eq!(
            fs::read(content.join("assets/game.js")).unwrap(),
            b"window.gameLoaded = true"
        );
        assert!(!content.join("Arcade").exists());
        fs::write(profile.join("save-marker"), b"keep progress").unwrap();
        let second = library.import_with_result(&source).unwrap();
        assert!(second.already_present);
        assert_eq!(second.game.id, first.game.id);
        assert_eq!(
            fs::read(profile.join("save-marker")).unwrap(),
            b"keep progress"
        );
        assert_eq!(library.list().unwrap().len(), 1);
    }

    #[test]
    fn wrapped_zip_without_explicit_directory_entries_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("wrapped.zip");
        archive(
            &source,
            &[
                ("Game/index.html", b"<script src=\"js/start.js\"></script>"),
                ("Game/js/start.js", b"window.started = true"),
            ],
        );
        let library = Library::open(dir.path().join("library")).unwrap();
        let game = library.import(&source).unwrap();
        let (content, _) = library.player_paths(game.id).unwrap();
        assert_eq!(
            fs::read(content.join("js/start.js")).unwrap(),
            b"window.started = true"
        );
        assert!(content.join("index.html").is_file());
    }

    #[test]
    fn wrapped_zip_rejects_ambiguous_and_unsafe_layouts_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let existing_source = dir.path().join("existing.html");
        fs::write(&existing_source, b"<html>existing</html>").unwrap();
        let existing = library.import(&existing_source).unwrap();
        let (_, profile) = library.player_paths(existing.id).unwrap();
        fs::write(profile.join("save-marker"), b"keep progress").unwrap();

        let source = dir.path().join("rejected.zip");
        for (case, entries) in [
            (
                "two wrappers",
                &[
                    ("One/index.html", b"one".as_slice()),
                    ("Two/index.html", b"two"),
                ][..],
            ),
            (
                "root and wrapper",
                &[
                    ("index.html", b"root".as_slice()),
                    ("Game/index.html", b"nested"),
                ][..],
            ),
            (
                "outside wrapper",
                &[
                    ("Game/index.html", b"game".as_slice()),
                    ("readme.txt", b"extra"),
                ][..],
            ),
            (
                "nested wrapper",
                &[("Outer/Inner/index.html", b"too deep".as_slice())][..],
            ),
            (
                "case collision",
                &[
                    ("Game/index.html", b"one".as_slice()),
                    ("game/INDEX.HTML", b"two"),
                ][..],
            ),
            (
                "traversal",
                &[
                    ("Game/index.html", b"game".as_slice()),
                    ("Game/../escape.txt", b"bad"),
                ][..],
            ),
            (
                "absolute path",
                &[
                    ("Game/index.html", b"game".as_slice()),
                    ("/escape.txt", b"bad"),
                ][..],
            ),
        ] {
            archive(&source, entries);
            assert!(library.import(&source).is_err(), "{case}");
            assert_eq!(library.list().unwrap(), vec![existing.clone()], "{case}");
            assert_eq!(
                fs::read(profile.join("save-marker")).unwrap(),
                b"keep progress",
                "{case}"
            );
            assert_eq!(
                fs::read_dir(root.join("incoming")).unwrap().count(),
                0,
                "{case}"
            );
            assert_eq!(
                fs::read_dir(root.join("games")).unwrap().count(),
                1,
                "{case}"
            );
        }
        assert!(!dir.path().join("escape.txt").exists());
    }

    fn html_game(dir: &Path, library: &Library, name: &str, body: &[u8]) -> Game {
        let source = dir.join(name);
        fs::write(&source, body).unwrap();
        library.import(&source).unwrap()
    }

    #[test]
    fn a_corrupt_game_folder_is_reported_without_hiding_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let good = html_game(dir.path(), &library, "Good.html", b"<html>good</html>");
        let broken = root.join("games").join(Uuid::new_v4().to_string());
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("game.json"), b"{ not json").unwrap();
        let mismatched = root.join("games").join(Uuid::new_v4().to_string());
        fs::create_dir_all(&mismatched).unwrap();
        fs::write(
            mismatched.join("game.json"),
            fs::read(
                root.join("games")
                    .join(good.id.to_string())
                    .join("game.json"),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(library.list().unwrap(), vec![good.clone()]);
        let scan = library.scan().unwrap();
        assert_eq!(scan.games.len(), 1);
        assert_eq!(scan.problems.len(), 2);
        assert!(
            broken.join("game.json").exists(),
            "problems are never deleted"
        );
    }

    #[test]
    fn missing_entry_files_are_flagged_and_cannot_launch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = html_game(dir.path(), &library, "Gone.html", b"<html>gone</html>");
        assert!(library.scan().unwrap().games[0].files_ok);
        fs::remove_file(
            root.join("games")
                .join(game.id.to_string())
                .join("content/index.html"),
        )
        .unwrap();
        let entry = &library.scan().unwrap().games[0];
        assert!(!entry.files_ok);
        let error = library.player_paths(game.id).unwrap_err().to_string();
        assert!(error.contains("missing"), "{error}");

        // Re-adding the original file is reported as a duplicate, so remove and re-add instead.
        library.remove(game.id, true).unwrap();
        assert!(library.list().unwrap().is_empty());
    }

    #[test]
    fn remove_without_saves_deletes_files_and_profile() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let keep = html_game(dir.path(), &library, "Keep.html", b"<html>keep</html>");
        let game = html_game(dir.path(), &library, "Drop.html", b"<html>drop</html>");
        let (_, profile) = library.player_paths(game.id).unwrap();
        fs::write(profile.join("save-marker"), b"progress").unwrap();
        library.remove(game.id, false).unwrap();
        assert_eq!(library.list().unwrap(), vec![keep]);
        assert!(!profile.exists());
        assert!(!root.join("games").join(game.id.to_string()).exists());
        assert_eq!(fs::read_dir(root.join("incoming")).unwrap().count(), 0);
        assert!(
            library.remove(game.id, false).is_err(),
            "second removal is an error"
        );
        let again = library.import(dir.path().join("Drop.html")).unwrap();
        assert_ne!(again.id, game.id, "deleted saves are not reattached");
    }

    #[test]
    fn remove_with_kept_saves_restores_them_when_the_same_file_returns() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = html_game(dir.path(), &library, "Saved.html", b"<html>saved</html>");
        let (_, profile) = library.player_paths(game.id).unwrap();
        fs::write(profile.join("save-marker"), b"level 9").unwrap();
        library.remove(game.id, true).unwrap();
        assert!(library.list().unwrap().is_empty());
        assert_eq!(fs::read(profile.join("save-marker")).unwrap(), b"level 9");

        // A different file does not pick up the retained profile.
        let other = html_game(dir.path(), &library, "Other.html", b"<html>other</html>");
        assert_ne!(other.id, game.id);

        let back = library
            .import_with_result(dir.path().join("Saved.html"))
            .unwrap();
        assert!(back.restored_saves);
        assert_eq!(back.game.id, game.id);
        let (_, restored) = library.player_paths(back.game.id).unwrap();
        assert_eq!(fs::read(restored.join("save-marker")).unwrap(), b"level 9");
        assert!(!library.retained_marker(game.id).exists());
    }

    #[test]
    fn reset_saves_clears_only_that_game() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let one = html_game(dir.path(), &library, "One.html", b"<html>1</html>");
        let two = html_game(dir.path(), &library, "Two.html", b"<html>2</html>");
        let (_, one_profile) = library.player_paths(one.id).unwrap();
        let (_, two_profile) = library.player_paths(two.id).unwrap();
        fs::write(one_profile.join("save"), b"one").unwrap();
        fs::write(two_profile.join("save"), b"two").unwrap();
        assert!(
            library
                .scan()
                .unwrap()
                .games
                .iter()
                .all(|game| game.save_bytes == 3)
        );
        library.reset_saves(one.id).unwrap();
        assert!(!one_profile.join("save").exists());
        assert_eq!(fs::read(two_profile.join("save")).unwrap(), b"two");
        library.reset_saves(one.id).unwrap();
        assert_eq!(library.list().unwrap().len(), 2);
    }

    #[test]
    fn a_running_game_cannot_be_removed_reset_or_opened_twice() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("library")).unwrap();
        let game = html_game(dir.path(), &library, "Busy.html", b"<html>busy</html>");
        let lock = library.lock_game(game.id).unwrap();
        assert!(library.lock_game(game.id).is_err());
        assert!(library.reset_saves(game.id).is_err());
        assert!(library.remove(game.id, false).is_err());
        assert_eq!(library.list().unwrap(), vec![game.clone()]);
        drop(lock);
        let relock = library.lock_game(game.id).unwrap();
        drop(relock);
        library.remove(game.id, false).unwrap();
    }

    #[test]
    fn rename_and_play_history_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = html_game(dir.path(), &library, "index.html", b"<html>x</html>");
        for bad in ["", "   ", &"x".repeat(81)] {
            assert!(library.rename(game.id, bad).is_err(), "{bad:?}");
        }
        assert!(library.rename(Uuid::new_v4(), "Nope").is_err());
        assert_eq!(
            library.rename(game.id, "  Star\u{7}field  ").unwrap().title,
            "Starfield"
        );
        library.record_play(game.id).unwrap();
        library.record_play(game.id).unwrap();
        let reopened = Library::open(&root).unwrap();
        let entry = &reopened.scan().unwrap().games[0];
        assert_eq!(entry.game.title, "Starfield");
        assert_eq!(entry.play_count, 2);
        assert!(entry.last_played.is_some());
        assert!(entry.game.added_at.is_some());
        let leftovers = fs::read_dir(root.join("games").join(game.id.to_string()))
            .unwrap()
            .filter(|item| {
                item.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn zip_slip_and_malformed_sources_are_rejected_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        for (name, entries) in [
            (
                "slip.zip",
                &[
                    ("index.html", b"ok".as_slice()),
                    ("assets/../../../outside.txt", b"bad"),
                ][..],
            ),
            (
                "drive.zip",
                &[
                    ("index.html", b"ok".as_slice()),
                    ("C:/Windows/outside.txt", b"bad"),
                ][..],
            ),
            (
                "device.zip",
                &[("index.html", b"ok".as_slice()), ("NUL.js", b"bad")][..],
            ),
        ] {
            archive(&dir.path().join(name), entries);
            assert!(library.import(dir.path().join(name)).is_err(), "{name}");
        }
        fs::write(
            dir.path().join("garbage.zip"),
            b"PK\x03\x04 this is not a zip",
        )
        .unwrap();
        assert!(library.import(dir.path().join("garbage.zip")).is_err());
        fs::write(dir.path().join("empty.html"), b"").unwrap();
        assert!(library.import(dir.path().join("empty.html")).is_err());
        fs::write(dir.path().join("game.exe"), b"MZ").unwrap();
        assert!(library.import(dir.path().join("game.exe")).is_err());
        assert!(
            library
                .import(dir.path().join("does-not-exist.zip"))
                .is_err()
        );
        assert!(library.list().unwrap().is_empty());
        assert_eq!(fs::read_dir(root.join("incoming")).unwrap().count(), 0);
        assert!(!dir.path().join("outside.txt").exists());
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into())
        }
    }

    #[test]
    fn a_profile_held_open_by_webview2_counts_as_busy() {
        assert!(profile_busy(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!profile_busy(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!profile_busy(&io::Error::from(io::ErrorKind::InvalidData)));
        assert_eq!(
            profile_busy(&io::Error::from_raw_os_error(32)),
            cfg!(windows)
        );
        let dir = tempfile::tempdir().unwrap();
        remove_profile(&dir.path().join("never-created")).unwrap();
        let profile = dir.path().join("profile");
        fs::create_dir_all(profile.join("Default")).unwrap();
        fs::write(profile.join("Default").join("Local State"), b"synthetic").unwrap();
        remove_profile(&profile).unwrap();
        assert!(!profile.exists());
    }

    #[test]
    fn default_library_locations_follow_each_os_convention() {
        // Windows keeps the original location, so existing libraries and saves are found again.
        assert_eq!(
            default_library_root_for(
                HostOs::Windows,
                env_of(&[("LOCALAPPDATA", r"C:\Users\Test\AppData\Local")])
            )
            .unwrap(),
            PathBuf::from(r"C:\Users\Test\AppData\Local")
                .join("GameStrike")
                .join("Library")
        );
        assert!(default_library_root_for(HostOs::Windows, env_of(&[])).is_err());
        assert!(
            default_library_root_for(HostOs::Windows, env_of(&[("LOCALAPPDATA", "relative")]))
                .is_err()
        );

        assert_eq!(
            default_library_root_for(HostOs::MacOs, env_of(&[("HOME", "/Users/test")])).unwrap(),
            PathBuf::from("/Users/test")
                .join("Library")
                .join("Application Support")
                .join("GameStrike")
                .join("Library")
        );
        assert!(default_library_root_for(HostOs::MacOs, env_of(&[])).is_err());

        assert_eq!(
            default_library_root_for(
                HostOs::Unix,
                env_of(&[("XDG_DATA_HOME", "/data/test"), ("HOME", "/home/test")])
            )
            .unwrap(),
            PathBuf::from("/data/test")
                .join("gamestrike")
                .join("library")
        );
        // A relative XDG_DATA_HOME is invalid by the spec and falls back to HOME.
        for env in [
            env_of(&[("HOME", "/home/test")]),
            env_of(&[("XDG_DATA_HOME", "data"), ("HOME", "/home/test")]),
        ] {
            assert_eq!(
                default_library_root_for(HostOs::Unix, env).unwrap(),
                PathBuf::from("/home/test")
                    .join(".local")
                    .join("share")
                    .join("gamestrike")
                    .join("library")
            );
        }
        assert!(default_library_root_for(HostOs::Unix, env_of(&[("HOME", "home")])).is_err());
        assert!(default_library_root().is_ok() || std::env::var_os("HOME").is_none());
    }

    #[test]
    fn engine_data_outside_the_library_is_cleared_only_on_macos() {
        let dir = tempfile::tempdir().unwrap();
        let result = clear_engine_data(dir.path(), Uuid::new_v4());
        if HostOs::current() == HostOs::MacOs {
            // The test binary has no gamestrike-player beside it, so this must fail, not pretend.
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }
        let player = sibling_player().unwrap();
        assert!(
            player
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("gamestrike-player")
        );
    }

    /// A synthetic test image: a two-color gradient, encoded as `format`.
    fn test_image(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let pixels = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                (x * 255 / width.max(1)) as u8,
                200,
                (y * 255 / height.max(1)) as u8,
                255,
            ])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(pixels).to_rgb8())
            .write_to(&mut io::Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let word = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        (word(16), word(20))
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn covers_are_decoded_scaled_and_reencoded_as_png() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
        ] {
            let png = cover_png_from_bytes(&test_image(64, 96, format)).unwrap();
            assert_eq!(png_size(&png), (64, 96), "{format:?}");
        }
        // Large images are scaled down to fit 600 × 900, keeping their shape.
        let (width, height) = png_size(
            &cover_png_from_bytes(&test_image(2400, 1200, image::ImageFormat::Png)).unwrap(),
        );
        assert!(
            width <= COVER_WIDTH && height <= COVER_HEIGHT,
            "{width}x{height}"
        );
        assert_eq!((width, height), (600, 300));
    }

    #[test]
    fn hostile_cover_files_are_refused() {
        for bytes in [
            &b""[..],
            b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>",
            b"<!doctype html><title>not an image</title>",
            b"\x89PNG\r\n\x1a\n truncated",
            b"GIF89a broken",
        ] {
            assert!(cover_png_from_bytes(bytes).is_err(), "{bytes:?}");
        }
        // A valid PNG header announcing a 9000 × 9000 image is refused before decoding pixels.
        let mut bomb = test_image(4, 4, image::ImageFormat::Png);
        bomb[16..20].copy_from_slice(&9000u32.to_be_bytes());
        bomb[20..24].copy_from_slice(&9000u32.to_be_bytes());
        let crc = crc32(&bomb[12..29]);
        bomb[29..33].copy_from_slice(&crc.to_be_bytes());
        let error = cover_png_from_bytes(&bomb).unwrap_err().to_string();
        assert!(
            error.contains("too large") || error.contains("damaged"),
            "{error}"
        );
        // Files over the size cap are refused without being read in full.
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.png");
        File::create(&big)
            .unwrap()
            .set_len(MAX_COVER_SOURCE_BYTES + 1)
            .unwrap();
        assert!(cover_png_from_file(&big).is_err());
        assert!(
            cover_png_from_file(dir.path()).is_err(),
            "a folder is not a cover"
        );
    }

    #[test]
    fn custom_covers_can_be_set_undone_and_reset() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = html_game(
            dir.path(),
            &library,
            "Cover Test.html",
            b"<html>covers</html>",
        );
        assert_eq!(library.scan().unwrap().games[0].cover, None);
        assert!(library.cover_file(game.id).is_none());
        assert!(library.undo_cover(game.id).is_err(), "nothing to undo yet");

        let first = dir.path().join("first.png");
        fs::write(&first, test_image(30, 45, image::ImageFormat::Png)).unwrap();
        let second = dir.path().join("second.jpg");
        fs::write(&second, test_image(50, 75, image::ImageFormat::Jpeg)).unwrap();
        library.set_cover(game.id, &first).unwrap();
        let entry = &library.scan().unwrap().games[0];
        assert_eq!(entry.cover, Some("custom"));
        assert!(!entry.cover_undo);
        let stored = library.cover_file(game.id).unwrap();
        assert!(
            stored.starts_with(root.join("games")),
            "covers live in the library"
        );
        assert_eq!(png_size(&fs::read(&stored).unwrap()), (30, 45));

        library.set_cover(game.id, &second).unwrap();
        assert_eq!(
            png_size(&fs::read(library.cover_file(game.id).unwrap()).unwrap()),
            (50, 75)
        );
        assert!(library.scan().unwrap().games[0].cover_undo);
        library.undo_cover(game.id).unwrap();
        assert_eq!(
            png_size(&fs::read(library.cover_file(game.id).unwrap()).unwrap()),
            (30, 45)
        );
        library.undo_cover(game.id).unwrap();
        assert_eq!(
            png_size(&fs::read(library.cover_file(game.id).unwrap()).unwrap()),
            (50, 75)
        );

        library.reset_cover(game.id).unwrap();
        assert_eq!(library.scan().unwrap().games[0].cover, None);
        library.undo_cover(game.id).unwrap();
        assert_eq!(library.scan().unwrap().games[0].cover, Some("custom"));

        // A rejected image leaves the current cover alone, and nothing is staged.
        let bad = dir.path().join("bad.png");
        fs::write(&bad, b"<html>not an image</html>").unwrap();
        assert!(library.set_cover(game.id, &bad).is_err());
        assert_eq!(
            png_size(&fs::read(library.cover_file(game.id).unwrap()).unwrap()),
            (50, 75)
        );
        let leftovers = fs::read_dir(root.join("games").join(game.id.to_string()))
            .unwrap()
            .filter(|item| {
                item.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
        assert!(library.set_cover(Uuid::new_v4(), &first).is_err());
    }

    #[test]
    fn a_game_icon_becomes_its_automatic_cover_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let source = dir.path().join("Icon Game.zip");
        let icon = test_image(128, 128, image::ImageFormat::Png);
        archive(
            &source,
            &[
                ("index.html", b"<html>icon</html>".as_slice()),
                ("icon.png", icon.as_slice()),
            ],
        );
        let game = library.import(&source).unwrap();
        let entry = &library.scan().unwrap().games[0];
        assert_eq!(entry.cover, Some("auto"));
        assert_eq!(
            png_size(&fs::read(library.cover_file(game.id).unwrap()).unwrap()),
            (128, 128)
        );
        // The player's own cover wins over the automatic one, and Reset returns to it.
        let custom = dir.path().join("custom.png");
        fs::write(&custom, test_image(20, 30, image::ImageFormat::Png)).unwrap();
        library.set_cover(game.id, &custom).unwrap();
        assert_eq!(library.scan().unwrap().games[0].cover, Some("custom"));
        library.reset_cover(game.id).unwrap();
        assert_eq!(library.scan().unwrap().games[0].cover, Some("auto"));

        // A game whose "icon" is not an image gets no automatic cover, and is searched once.
        let broken = dir.path().join("Broken Icon.zip");
        archive(
            &broken,
            &[
                ("index.html", b"<html>broken</html>".as_slice()),
                ("icon.png", b"<script>x</script>".as_slice()),
            ],
        );
        let broken = library.import(&broken).unwrap();
        assert!(library.cover_file(broken.id).is_none());
        assert!(
            root.join("games")
                .join(broken.id.to_string())
                .join(AUTO_COVER_SEARCHED)
                .exists()
        );
        library.ensure_auto_cover(broken.id).unwrap();
        assert!(library.cover_file(broken.id).is_none());
    }

    #[test]
    fn only_a_marked_library_counts_as_synthetic_test_data() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path()).unwrap();
        assert!(!library.is_synthetic_test_library());
        fs::create_dir(dir.path().join("SYNTHETIC-TEST-LIBRARY")).unwrap();
        assert!(
            !library.is_synthetic_test_library(),
            "a folder is not the marker"
        );
        fs::remove_dir(dir.path().join("SYNTHETIC-TEST-LIBRARY")).unwrap();
        fs::write(dir.path().join("SYNTHETIC-TEST-LIBRARY"), b"").unwrap();
        assert!(library.is_synthetic_test_library());
    }

    #[test]
    fn source_and_favorite_are_saved_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let library = Library::open(&root).unwrap();
        let game = html_game(dir.path(), &library, "Tagged.html", b"<html>tagged</html>");
        assert_eq!(game.source, None);
        assert!(!game.favorite);
        assert_eq!(
            library
                .set_source(game.id, Some("  itch\u{7}.io "))
                .unwrap()
                .source
                .as_deref(),
            Some("itch.io")
        );
        assert!(library.set_source(game.id, Some(&"x".repeat(33))).is_err());
        assert!(library.set_favorite(game.id, true).unwrap().favorite);
        let reopened = Library::open(&root).unwrap();
        let listed = &reopened.list().unwrap()[0];
        assert_eq!(listed.source.as_deref(), Some("itch.io"));
        assert!(listed.favorite);
        assert_eq!(
            library.set_source(game.id, Some("   ")).unwrap().source,
            None
        );
        assert_eq!(library.set_source(game.id, None).unwrap().source, None);
        assert!(library.set_favorite(Uuid::new_v4(), true).is_err());

        // Manifests written before these fields existed still load.
        let manifest = root
            .join("games")
            .join(game.id.to_string())
            .join("game.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("favorite");
        value.as_object_mut().unwrap().remove("source");
        fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let legacy = &library.list().unwrap()[0];
        assert_eq!((legacy.source.clone(), legacy.favorite), (None, false));
    }

    #[test]
    fn hostile_paths_modes_and_actual_expansion_are_rejected() {
        for path in [
            "/index.html",
            "C:/index.html",
            "..\\index.html",
            "../index.html",
            "CON.txt",
            "folder//index.html",
            "folder./index.html",
        ] {
            assert!(safe_content_path(path).is_err(), "{path}");
        }
        assert!(validate_mode(Some(0o120777), false).is_err());
        let mut output = Vec::new();
        let mut total = 0;
        assert!(copy_bounded(&mut &b"12345"[..], &mut output, &mut total, 3).is_err());
    }
}
