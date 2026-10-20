use gamestrike::{Library, clear_engine_data};
use std::{env, error::Error, io, path::PathBuf};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() < 4 || args.get(1).and_then(|arg| arg.to_str()) != Some("--library") {
        return Err(usage().into());
    }
    let root = PathBuf::from(&args[2]);
    let library = Library::open(&root)?;
    let id = |index: usize| -> Result<Uuid, Box<dyn Error>> {
        Ok(Uuid::parse_str(
            args.get(index)
                .and_then(|arg| arg.to_str())
                .ok_or_else(usage)?,
        )?)
    };
    match args[3].to_str() {
        Some("import") if args.len() == 5 => {
            let outcome = library.import_with_result(PathBuf::from(&args[4]))?;
            println!(
                "{} {} ({})",
                if outcome.already_present {
                    "Already in library:"
                } else if outcome.restored_saves {
                    "Imported with restored saves:"
                } else {
                    "Imported"
                },
                outcome.game.title,
                outcome.game.id
            );
        }
        Some("list") if args.len() == 4 => {
            let scan = library.scan()?;
            for entry in scan.games {
                println!(
                    "{}\t{}\tfiles: {}\tplays: {}\tsave bytes: {}\tcontroller: unknown\tnetwork: unknown\tsource: {}\tfavorite: {}\tcover: {}",
                    entry.game.id,
                    entry.game.title,
                    if entry.files_ok { "ok" } else { "missing" },
                    entry.play_count,
                    entry.save_bytes,
                    entry.game.source.as_deref().unwrap_or("-"),
                    if entry.game.favorite { "yes" } else { "no" },
                    entry.cover.unwrap_or("generated"),
                );
            }
            for problem in scan.problems {
                eprintln!("unreadable game folder: {problem}");
            }
        }
        Some("rename") if args.len() == 6 => {
            let title = args[5].to_str().ok_or("title is not UTF-8")?;
            println!("Renamed to {}", library.rename(id(4)?, title)?.title);
        }
        Some("reset-saves") if args.len() == 5 => {
            let game = id(4)?;
            // On macOS the WebKit data store lives outside the library and is cleared first.
            clear_engine_data(&root, game)?;
            library.reset_saves(game)?;
            println!("Saves cleared.");
        }
        Some("remove") if args.len() == 5 || args.len() == 6 => {
            let keep_saves = match args.get(5).and_then(|arg| arg.to_str()) {
                None => false,
                Some("--keep-saves") => true,
                Some(_) => return Err(usage().into()),
            };
            let game = id(4)?;
            if !keep_saves {
                clear_engine_data(&root, game)?;
            }
            library.remove(game, keep_saves)?;
            println!(
                "Removed{}.",
                if keep_saves {
                    "; saves kept for the same file"
                } else {
                    " with its saves"
                }
            );
        }
        Some("set-source") if args.len() == 6 => {
            let source = args[5].to_str().ok_or("source is not UTF-8")?;
            let source = (source != "--clear").then_some(source);
            let game = library.set_source(id(4)?, source)?;
            println!("Source: {}", game.source.as_deref().unwrap_or("none"));
        }
        Some("favorite") if args.len() == 6 => {
            let favorite = match args[5].to_str() {
                Some("on") => true,
                Some("off") => false,
                _ => return Err(usage().into()),
            };
            library.set_favorite(id(4)?, favorite)?;
            println!("Favorite: {}", if favorite { "on" } else { "off" });
        }
        Some("set-cover") if args.len() == 6 => {
            library.set_cover(id(4)?, PathBuf::from(&args[5]))?;
            println!("Cover set.");
        }
        Some("undo-cover") if args.len() == 5 => {
            library.undo_cover(id(4)?)?;
            println!("Previous cover restored.");
        }
        Some("reset-cover") if args.len() == 5 => {
            library.reset_cover(id(4)?)?;
            println!("Cover reset; undo-cover brings it back.");
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: gamestrike --library DIRECTORY import GAME.html|GAME.zip\n       gamestrike --library DIRECTORY list\n       gamestrike --library DIRECTORY rename GAME_UUID TITLE\n       gamestrike --library DIRECTORY reset-saves GAME_UUID\n       gamestrike --library DIRECTORY remove GAME_UUID [--keep-saves]\n       gamestrike --library DIRECTORY set-source GAME_UUID SOURCE|--clear\n       gamestrike --library DIRECTORY favorite GAME_UUID on|off\n       gamestrike --library DIRECTORY set-cover GAME_UUID IMAGE\n       gamestrike --library DIRECTORY undo-cover GAME_UUID\n       gamestrike --library DIRECTORY reset-cover GAME_UUID",
    )
}
