use super::{AppSnapshot, CatalogBackend, CoverImage, GameCard, GameId, GameOrigin, LaunchOutcome};
use crate::{
    Library,
    music::{MusicCommand, MusicSnapshot, MusicStore},
    stores::{Launch, StoreLibrary},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// OS adapters are injected; this safe library does not contain platform FFI or window handles.
pub trait DesktopHost: Send {
    fn refresh_stores(&mut self, stores: &StoreLibrary) -> Result<(), String>;
    fn launch_store(&mut self, target: &Launch) -> Result<LaunchOutcome, String>;
    fn launch_html(&mut self, root: &Path, id: Uuid) -> Result<LaunchOutcome, String>;
}

pub struct LibraryBackend<H: DesktopHost> {
    root: PathBuf,
    library: Library,
    stores: StoreLibrary,
    host: H,
    music: Result<MusicStore, String>,
}

impl<H: DesktopHost> LibraryBackend<H> {
    pub fn open(root: PathBuf, host: H) -> Result<Self, String> {
        let library = Library::open(&root).map_err(|error| error.to_string())?;
        let stores = StoreLibrary::open(&root).map_err(|error| error.to_string())?;
        let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
        // A damaged music manifest disables only Music, preserving the game collection.
        let music = MusicStore::open(&root);
        Ok(Self {
            root,
            library,
            stores,
            host,
            music,
        })
    }
}

impl<H: DesktopHost> CatalogBackend for LibraryBackend<H> {
    fn snapshot(&mut self) -> Result<AppSnapshot, String> {
        let scan = self
            .library
            .scan()
            .map_err(|error| format!("Library could not be read: {error}"))?;
        let entries = self
            .stores
            .try_entries()
            .map_err(|error| format!("Store catalog could not be read: {error}"))?;
        // Thumbnail storage is bounded independently of the number of installed games.
        let mut cover_budget = 24 * 1024 * 1024_usize;
        let mut games = Vec::with_capacity(scan.games.len() + entries.len());
        for entry in scan.games {
            let cover = thumbnail(
                self.library.display_cover_file(entry.game.id),
                &mut cover_budget,
            );
            games.push(GameCard {
                id: GameId::Html(entry.game.id),
                origin: GameOrigin::Html,
                title: entry.game.title,
                source: entry.game.source.unwrap_or_else(|| "HTML".into()),
                favorite: entry.game.favorite,
                files_ok: entry.files_ok,
                last_played: entry.last_played,
                cover,
            });
        }
        for entry in entries.into_iter().filter(|entry| !entry.hidden) {
            let cover = thumbnail(self.stores.cover_file(entry.id), &mut cover_budget);
            games.push(GameCard {
                id: GameId::Store(entry.id.to_string()),
                origin: GameOrigin::Store(entry.store),
                title: entry.title,
                source: entry.store_name.into(),
                favorite: entry.favorite,
                files_ok: entry.installed,
                last_played: entry.last_played,
                cover,
            });
        }
        games.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        Ok(AppSnapshot {
            games,
            providers: self.stores.statuses(crate::HostOs::current()),
            synthetic: false,
            music: match &self.music {
                Ok(store) => store.snapshot(),
                Err(error) => MusicSnapshot {
                    problem: Some(format!("Music could not be read: {error}")),
                    ..Default::default()
                },
            },
            problems: scan.problems,
            running: BTreeSet::new(),
            refreshing: false,
            revision: 0,
        })
    }

    fn refresh(&mut self) -> Result<(), String> {
        self.host.refresh_stores(&self.stores)
    }

    fn update_music(&mut self, command: MusicCommand) -> Result<MusicSnapshot, String> {
        self.music
            .as_mut()
            .map_err(|error| error.clone())?
            .apply(command)
    }

    fn import(&mut self, source: &Path) -> Result<(), String> {
        self.library
            .import_with_result(source)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn set_favorite(&mut self, id: &GameId, favorite: bool) -> Result<(), String> {
        match id {
            GameId::Html(id) => self.library.set_favorite(*id, favorite).map(|_| ()),
            GameId::Store(id) => self.stores.set_favorite(store_id(id)?, favorite),
        }
        .map_err(|error| error.to_string())
    }

    fn launch(&mut self, id: &GameId) -> Result<LaunchOutcome, String> {
        match id {
            GameId::Html(id) => {
                self.library
                    .player_paths(*id)
                    .map_err(|error| error.to_string())?;
                self.host.launch_html(&self.root, *id)
            }
            GameId::Store(id) => {
                let id = store_id(id)?;
                let (target, _) = self
                    .stores
                    .launch_target(id)
                    .map_err(|error| error.to_string())?;
                // Revalidate immediately before passing a target to the OS adapter.
                crate::stores::check_launch(&target).map_err(|error| error.to_string())?;
                let result = self.host.launch_store(&target)?;
                // A metadata write failure must not turn a successful process start into a retry.
                if let Err(error) = self.stores.record_play(id) {
                    eprintln!(
                        "GameStrike started the game, but activity could not be saved: {error}"
                    );
                }
                Ok(result)
            }
        }
    }
}

fn store_id(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| "Invalid store game ID".into())
}

fn thumbnail(path: Option<PathBuf>, remaining: &mut usize) -> Option<CoverImage> {
    const MAX_THUMBNAIL_BYTES: usize = 160 * 240 * 4;
    if *remaining < MAX_THUMBNAIL_BYTES {
        return None;
    }
    let path = path?;
    let image = crate::decode_cover_file(&path)
        .ok()?
        .thumbnail(160, 240)
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let rgba = image.into_raw();
    *remaining = remaining.saturating_sub(rgba.len());
    Some(CoverImage { size, rgba })
}
