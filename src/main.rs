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
                    "{}\t{}\tfiles: {}\tplays: {}\tsave bytes: {}\tcontroller: unknown\tnetwork: unknown",
                    entry.game.id,
                    entry.game.title,
                    if entry.files_ok { "ok" } else { "missing" },
                    entry.play_count,
                    entry.save_bytes
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
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: gamestrike --library DIRECTORY import GAME.html|GAME.zip\n       gamestrike --library DIRECTORY list\n       gamestrike --library DIRECTORY rename GAME_UUID TITLE\n       gamestrike --library DIRECTORY reset-saves GAME_UUID\n       gamestrike --library DIRECTORY remove GAME_UUID [--keep-saves]",
    )
}
