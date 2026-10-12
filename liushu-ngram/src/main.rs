use clap::{Parser, Subcommand};

use liushu_core::{engine::Engine, lm::train};

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Train a word-level bigram language model on a text corpus
    Train {
        #[arg(short, long)]
        corpus_path: String,

        #[arg(short, long)]
        output_path: String,
    },

    /// Decode pinyin into the most probable sentence using a dictionary and a trained model
    Lookup {
        /// 拼音，连着写或者按音节分开都行：`lookup nihao` / `lookup ni hao`
        inputs: Vec<String>,

        #[arg(short, long)]
        model_path: String,

        /// 词库，与模型一起决定每个拼音能是哪些词
        #[arg(short = 'd', long)]
        dict_path: String,

        /// 返回几条候选
        #[arg(short, long, default_value_t = 1)]
        count: usize,
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
        Some(Commands::Lookup {
            inputs,
            model_path,
            dict_path,
            count,
        }) => {
            let engine = Engine::with_model(dict_path, model_path).unwrap();
            // 音节之间可以有空格，拼起来就是词库里那种连着写的编码。
            let code: String = inputs.concat();
            let sentences = engine.sentences(&code, *count);
            if sentences.is_empty() {
                eprintln!("no sentence for {code:?}");
                std::process::exit(1);
            }
            for (rank, sentence) in sentences.iter().enumerate() {
                println!("{}. {}  {:.2}", rank + 1, sentence.text, sentence.score);
            }
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
