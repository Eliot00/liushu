use std::io::{stdin, stdout, Write};

use clap::{Parser, Subcommand};
use liushu_core::deploy::deploy;
use liushu_core::dirs::PROJECT_DIRS;
use liushu_core::engine::{
    state::State, EngineManager, InputMethodEngine, NewEngine, ShapeCodeEngine,
};
use liushu_core::hmm::{train, Hmm};
use redb::Database;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Deploy,

    #[command(arg_required_else_help = true)]
    Train {
        corpus_file: String,
    },

    Repl,
}

fn main() {
    let args = Cli::parse();

    match args.command {
        Commands::Deploy => {
            deploy();
        }
        Commands::Train { corpus_file } => {
            let save_to = &PROJECT_DIRS.target_dir.join("hmm_model.redb");
            train(corpus_file, save_to);
        }
        Commands::Repl => {
            let db = Database::open(PROJECT_DIRS.target_dir.join("hmm_model.redb")).unwrap();
            let pinyin = Hmm::new(db);

            let sunman = ShapeCodeEngine::default();
            let mut engine_manager = EngineManager::from(
                [Box::new(sunman), Box::new(pinyin)] as [Box<dyn InputMethodEngine>; 2]
            );

            loop {
                print!("liushu> ");
                stdout().flush().unwrap();
                let mut input = String::new();
                match stdin().read_line(&mut input) {
                    Ok(_) => {
                        let input = input.trim();

                        if input == "*shift" {
                            engine_manager.set_active_engine(1);
                        }

                        if input == "*quit" {
                            break;
                        }

                        engine_manager
                            .search(input)
                            .unwrap_or_else(|e| {
                                println!("error: {}", e);
                                vec![]
                            })
                            .iter()
                            .take(8)
                            .enumerate()
                            .for_each(|(i, result)| {
                                println!("result{}: {:?}", i, result);
                            });
                    }
                    Err(error) => println!("error: {}", error),
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::Cli;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert()
    }
}
