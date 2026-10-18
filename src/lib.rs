#![forbid(unsafe_code)]

//! Import-only foundation. This crate never interprets imported HTML or JavaScript.

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
            fs::remove_dir_all(&profile)?;
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
            fs::remove_dir_all(&profile)?;
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
