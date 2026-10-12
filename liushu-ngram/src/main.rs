use clap::{Parser, Subcommand};

use liushu_core::lm::{pinyin_to_sentence, train};

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Train a bigram language model on a text corpus
    Train {
        #[arg(short, long)]
        corpus_path: String,

        #[arg(short, long)]
        output_path: String,
    },

    /// Lookup the most probable hanzi sequence for the input using a trained model
    Lookup {
        inputs: Vec<String>,

        #[arg(short, long)]
        model_path: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Some(Commands::Train {
            corpus_path,
            output_path,
        }) => {
            train(corpus_path, output_path).unwrap();
        }
        Some(Commands::Lookup { inputs, model_path }) => {
            let result = pinyin_to_sentence(inputs, model_path).unwrap();
            println!("result is {}", result);
        }
        None => {}
    }
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
