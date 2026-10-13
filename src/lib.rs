#![forbid(unsafe_code)]

//! Import-only foundation. This crate never interprets imported HTML or JavaScript.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
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
}

pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("games"))?;
        fs::create_dir_all(root.join("incoming"))?;
        Ok(Self { root })
    }

    pub fn import(&self, source: impl AsRef<Path>) -> Result<Game> {
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
        let id = Uuid::new_v4();
        let game = Game {
            id,
            title: display_title(source),
            source_kind: kind.clone(),
            entry: "index.html".to_owned(),
            controller_support: "unknown".to_owned(),
            network_requirement: "unknown".to_owned(),
        };
        let stage = self.root.join("incoming").join(id.to_string());
        fs::create_dir(&stage)?;
        let result = (|| -> Result<()> {
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
            let mut manifest = new_file(&stage.join("game.json"))?;
            serde_json::to_writer_pretty(&mut manifest, &game)?;
            manifest.sync_all()?;
            drop(manifest);
            fs::rename(&stage, self.root.join("games").join(id.to_string()))?;
            Ok(())
        })();
        if result.is_err() {
            // Only this call's newly created UUID staging directory is removed.
            let _ = fs::remove_dir_all(&stage);
        }
        result?;
        Ok(game)
    }

    pub fn list(&self) -> Result<Vec<Game>> {
        let mut games = Vec::new();
        for item in fs::read_dir(self.root.join("games"))? {
            let item = item?;
            if !item.file_type()?.is_dir() {
                continue;
            }
            let path = item.path().join("game.json");
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_MANIFEST_BYTES
            {
                return Err(invalid("invalid game manifest").into());
            }
            let game: Game = serde_json::from_reader(File::open(path)?)?;
            let expected_name = game.id.to_string();
            if item.file_name().to_string_lossy().as_ref() != expected_name.as_str() {
                return Err(invalid("game directory and manifest ID differ").into());
            }
            games.push(game);
        }
        games.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        Ok(games)
    }

    /// Only generated game IDs can select content and persistent browser storage.
    pub fn player_paths(&self, id: Uuid) -> Result<(PathBuf, PathBuf)> {
        if !self.list()?.iter().any(|game| game.id == id) {
            return Err(invalid("game ID was not found in the library").into());
        }
        let content = self.root.join("games").join(id.to_string()).join("content");
        if !fs::symlink_metadata(&content)?.file_type().is_dir() {
            return Err(invalid("game content directory is invalid").into());
        }
        let profiles = self.root.join("profiles");
        fs::create_dir_all(&profiles)?;
        if !fs::symlink_metadata(&profiles)?.file_type().is_dir() {
            return Err(invalid("browser profile directory is invalid").into());
        }
        let profile = profiles.join(id.to_string());
        fs::create_dir_all(&profile)?;
        if !fs::symlink_metadata(&profile)?.file_type().is_dir() {
            return Err(invalid("game browser profile is invalid").into());
        }
        Ok((fs::canonicalize(content)?, fs::canonicalize(profile)?))
    }
}

fn import_zip(source: &Path, content: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(source)?)?;
    if archive.len() == 0 || archive.len() > MAX_FILES {
        return Err(invalid("ZIP has no entries or too many entries").into());
    }
    let mut entries = Vec::with_capacity(archive.len());
    let mut seen = HashSet::new();
    let mut regular_files = HashSet::new();
    let mut found_entry = false;
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
            if relative == Path::new("index.html") {
                found_entry = true;
            }
        }
        entries.push((relative, is_dir));
    }
    if !found_entry {
        return Err(invalid("ZIP needs a root index.html entry point").into());
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
        let path = content.join(relative);
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
    let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
    if title.trim().is_empty() {
        "Imported game".to_owned()
    } else {
        title
    }
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
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body).unwrap();
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
