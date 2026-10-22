//! Native local music queue metadata and playback services.
//! The store keeps source references; only an explicitly requested host opens media.

pub mod playback;
pub mod service;
pub mod streaming;

#[cfg(all(windows, feature = "native-music-host"))]
pub mod host;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;
const MAX_QUEUE: usize = 256;
const MAX_PLAYLISTS: usize = 32;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_NAME_CHARS: usize = 80;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: Uuid,
    pub title: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Playlist {
    pub id: Uuid,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MusicSnapshot {
    pub queue: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub available: bool,
    pub problem: Option<String>,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub enum MusicCommand {
    AddTracks(Vec<PathBuf>),
    MoveTrack { id: Uuid, up: bool },
    RemoveTrack(Uuid),
    SavePlaylist { name: String },
    LoadPlaylist(Uuid),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_version: u32,
    revision: u64,
    queue: Vec<Track>,
    playlists: Vec<Playlist>,
}

/// A single-owner store. The last-read bytes also detect external metadata changes
/// before writing; this is not a cross-process locking protocol.
pub struct MusicStore {
    root: PathBuf,
    snapshot: MusicSnapshot,
    persisted: Option<Vec<u8>>,
}

impl MusicStore {
    /// Reads only root/music.json. Missing metadata is an empty available store;
    /// malformed, unsupported or unreadable metadata is never reset or overwritten.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        let persisted = read_catalog_bytes(&root.join("music.json"))?;
        let snapshot = match &persisted {
            Some(bytes) => {
                let catalog: Catalog = serde_json::from_slice(bytes)
                    .map_err(|error| format!("Invalid music metadata: {error}"))?;
                validate_catalog(&catalog)?;
                MusicSnapshot {
                    queue: catalog.queue,
                    playlists: catalog.playlists,
                    available: true,
                    problem: None,
                    revision: catalog.revision,
                }
            }
            None => MusicSnapshot {
                available: true,
                ..Default::default()
            },
        };
        Ok(Self {
            root,
            snapshot,
            persisted,
        })
    }

    pub fn snapshot(&self) -> MusicSnapshot {
        self.snapshot.clone()
    }

    /// Publish only after validation and successful replacement of the metadata.
    /// Every rejected command/write leaves the previous runtime snapshot unchanged.
    pub fn apply(&mut self, command: MusicCommand) -> Result<MusicSnapshot, String> {
        let mut next = self.snapshot.clone();
        apply_command(&mut next, command)?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("Music revision limit reached")?;
        let catalog = Catalog {
            schema_version: SCHEMA_VERSION,
            revision: next.revision,
            queue: next.queue.clone(),
            playlists: next.playlists.clone(),
        };
        validate_catalog(&catalog)?;
        let bytes = serde_json::to_vec_pretty(&catalog)
            .map_err(|error| format!("Could not encode music metadata: {error}"))?;
        if bytes.len() > MAX_METADATA_BYTES {
            return Err("Music metadata exceeds the 1 MiB limit".into());
        }
        self.persist(&bytes)?;
        self.persisted = Some(bytes);
        self.snapshot = next;
        Ok(self.snapshot())
    }

    fn persist(&self, bytes: &[u8]) -> Result<(), String> {
        let path = self.root.join("music.json");
        if read_catalog_bytes(&path)? != self.persisted {
            return Err("Music metadata changed outside this store; reopen before saving".into());
        }
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("Could not create music metadata directory: {error}"))?;
        let temporary = self
            .root
            .join(format!("music.tmp-{}", Uuid::new_v4().simple()));
        // Never clean up a pre-existing path if create_new itself fails.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("Could not create music metadata temporary file: {error}"))?;
        let result = (|| -> std::io::Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|error| format!("Could not save music metadata: {error}"))
    }
}

fn apply_command(snapshot: &mut MusicSnapshot, command: MusicCommand) -> Result<(), String> {
    match command {
        MusicCommand::AddTracks(paths) => {
            if paths.is_empty() || paths.len() > MAX_QUEUE.saturating_sub(snapshot.queue.len()) {
                return Err("Choose between 1 and the remaining 256 queue entries".into());
            }
            for path in paths {
                validate_path(&path)?;
                let stem = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or("Chosen track path has no usable filename")?;
                let title: String = stem.trim().chars().take(MAX_NAME_CHARS).collect();
                validate_name(&title)?;
                snapshot.queue.push(Track {
                    id: Uuid::new_v4(),
                    title,
                    path,
                });
            }
        }
        MusicCommand::MoveTrack { id, up } => {
            let index = snapshot
                .queue
                .iter()
                .position(|track| track.id == id)
                .ok_or("Track is not in the queue")?;
            let destination = if up {
                index.checked_sub(1)
            } else {
                index
                    .checked_add(1)
                    .filter(|&next| next < snapshot.queue.len())
            }
            .ok_or("Track is already at that end of the queue")?;
            snapshot.queue.swap(index, destination);
        }
        MusicCommand::RemoveTrack(id) => {
            let index = snapshot
                .queue
                .iter()
                .position(|track| track.id == id)
                .ok_or("Track is not in the queue")?;
            snapshot.queue.remove(index);
        }
        MusicCommand::SavePlaylist { name } => {
            let name = name.trim().to_owned();
            validate_name(&name)?;
            if snapshot.playlists.len() >= MAX_PLAYLISTS {
                return Err("The 32 saved playlist limit has been reached".into());
            }
            if snapshot
                .playlists
                .iter()
                .any(|playlist| same_name(&playlist.name, &name))
            {
                return Err("A playlist with that name already exists".into());
            }
            snapshot.playlists.push(Playlist {
                id: Uuid::new_v4(),
                name,
                tracks: snapshot.queue.clone(),
            });
        }
        MusicCommand::LoadPlaylist(id) => {
            snapshot.queue = snapshot
                .playlists
                .iter()
                .find(|playlist| playlist.id == id)
                .ok_or("Saved playlist was not found")?
                .tracks
                .clone();
        }
    }
    Ok(())
}

fn validate_catalog(catalog: &Catalog) -> Result<(), String> {
    if catalog.schema_version != SCHEMA_VERSION {
        return Err("Unsupported music metadata schema version".into());
    }
    validate_tracks(&catalog.queue)?;
    if catalog.playlists.len() > MAX_PLAYLISTS {
        return Err("Music metadata exceeds 32 saved playlists".into());
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for playlist in &catalog.playlists {
        if playlist.id.is_nil() || !ids.insert(playlist.id) {
            return Err("Music metadata contains an invalid or duplicate playlist ID".into());
        }
        validate_name(&playlist.name)?;
        if !names.insert(playlist.name.to_lowercase()) {
            return Err("Music metadata contains duplicate playlist names".into());
        }
        validate_tracks(&playlist.tracks)?;
    }
    Ok(())
}

fn validate_tracks(tracks: &[Track]) -> Result<(), String> {
    if tracks.len() > MAX_QUEUE {
        return Err("Music metadata exceeds 256 tracks in a queue or saved playlist".into());
    }
    let mut ids = BTreeSet::new();
    for track in tracks {
        if track.id.is_nil() || !ids.insert(track.id) {
            return Err("Music metadata contains an invalid or duplicate track ID".into());
        }
        validate_name(&track.title)?;
        validate_path(&track.path)?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.trim() != name
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err("Music names must contain 1 to 80 trimmed, control-free characters".into());
    }
    Ok(())
}

fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

fn validate_path(path: &Path) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or("Chosen track path must be valid Unicode")?;
    if !path.is_absolute()
        || path.file_name().is_none()
        || text.len() > MAX_PATH_BYTES
        || text.chars().any(char::is_control)
    {
        return Err("Choose an absolute track filename of at most 4096 bytes without control characters".into());
    }
    Ok(())
}

fn read_catalog_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect music metadata: {error}")),
    };
    if !metadata.file_type().is_file() {
        return Err("Music metadata must be a regular file, not a directory or symbolic link".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| format!("Could not open music metadata: {error}"))?
        .take((MAX_METADATA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read music metadata: {error}"))?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err("Music metadata exceeds the 1 MiB limit".into());
    }
    Ok(Some(bytes))
}
