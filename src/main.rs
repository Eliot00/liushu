use std::io::{stdin, stdout, Write};

use clap::{Parser, Subcommand};
use liushu_core::deploy::deploy;
use liushu_core::dirs::PROJECT_DIRS;
use liushu_core::engine::{Engine, InputMethodEngine};
use liushu_core::hmm::train;

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
            deploy().unwrap();
        }
        Commands::Train { corpus_file } => {
            let save_to = &PROJECT_DIRS.target_dir.join("hmm_model.redb");
            train(corpus_file, save_to);
        }
        Commands::Repl => {
            let mut engine =
                Engine::init(&PROJECT_DIRS.data_dir, &PROJECT_DIRS.target_dir).unwrap();

            loop {
                print!("liushu> ");
                stdout().flush().unwrap();
                let mut input = String::new();
                match stdin().read_line(&mut input) {
                    Ok(_) => {
                        let input = input.trim();

                        if input.starts_with("*use") {
                            let formula_id = input.split(' ').last().unwrap();
                            engine.set_active_formula(formula_id).unwrap();
                        }

                        if input == "*quit" {
                            break;
                        }

                        engine
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
