use gamestrike::Library;
use std::{env, error::Error, io, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() < 4 || args.get(1).and_then(|arg| arg.to_str()) != Some("--library") {
        return Err(usage().into());
    }
    let root = PathBuf::from(&args[2]);
    let library = Library::open(root)?;
    match args[3].to_str() {
        Some("import") if args.len() == 5 => {
            let outcome = library.import_with_result(PathBuf::from(&args[4]))?;
            println!(
                "{} {} ({})",
                if outcome.already_present {
                    "Already in library:"
                } else {
                    "Imported"
                },
                outcome.game.title,
                outcome.game.id
            );
        }
        Some("list") if args.len() == 4 => {
            for game in library.list()? {
                println!(
                    "{}\t{}\tcontroller: unknown\tnetwork: unknown",
                    game.id, game.title
                );
            }
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: gamestrike --library DIRECTORY import GAME.html|GAME.zip\n       gamestrike --library DIRECTORY list",
    )
}
