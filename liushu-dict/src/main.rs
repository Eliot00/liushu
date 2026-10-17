//! `liushu-dict`：把码表 / 词表做成输入法直接 mmap 的词库容器。
//!
//! ```text
//! # TSV（首行表头 text\tcode\tweight[\tcomment]）
//! liushu-dict build -i dict.tsv -o sunman.trie
//!
//! # Rime 码表，import_tables 会自动跟着读
//! liushu-dict build -i ~/Library/Rime/shanren_sc.dict.yaml -o sunman.trie
//!
//! # 建好之后不用手机也能查
//! liushu-dict lookup -d sunman.trie cx
//! ```

mod rime;

use clap::{Parser, Subcommand};
use liushu_core::dict::{self, Dictionary};
use liushu_core::error::LiushuError;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a dictionary container from TSV files and/or Rime `.dict.yaml` tables
    Build {
        /// 输入文件：`.yaml` / `.yml` 按 Rime 码表解析，其余按 TSV（可重复给多个）
        #[arg(short, long, required = true)]
        inputs: Vec<String>,

        #[arg(short, long)]
        output: String,
    },

    /// Look up candidates for a code in a built dictionary
    Lookup {
        /// 要查的编码
        code: String,

        #[arg(short = 'd', long)]
        dict_path: String,

        /// 最多打印多少条
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
}

/// `.yaml` / `.yml` 当 Rime 码表，其余当 TSV。
fn is_rime(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.ends_with(".yaml") || path.ends_with(".yml")
}

fn read_rows(inputs: &[String]) -> Result<Vec<(Box<str>, dict::DictItem)>, LiushuError> {
    let mut rows = Vec::new();
    for input in inputs {
        rows.extend(if is_rime(input) {
            rime::read(input)?
        } else {
            dict::read_tsv(input)?
        });
    }
    Ok(rows)
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Some(Commands::Build { inputs, output }) => {
            let rows = match read_rows(inputs) {
                Ok(rows) => rows,
                Err(error) => {
                    eprintln!("读输入失败：{error}");
                    std::process::exit(1);
                }
            };
            println!("{} 条候选 → {output}", rows.len());
            if let Err(error) = dict::build_rows(rows, output) {
                eprintln!("建库失败：{error}");
                std::process::exit(1);
            }
        }
        Some(Commands::Lookup {
            code,
            dict_path,
            limit,
        }) => {
            let dictionary = match Dictionary::open(dict_path) {
                Ok(dictionary) => dictionary,
                Err(error) => {
                    eprintln!("打开词库失败：{error}");
                    std::process::exit(1);
                }
            };
            let candidates = dictionary.translate_limited(code, *limit);
            if candidates.is_empty() {
                println!("{code:?}: 没有候选");
                return;
            }
            for (rank, candidate) in candidates.iter().enumerate() {
                println!(
                    "{:>3}. {}  [{}]  权重 {}",
                    rank + 1,
                    candidate.text,
                    candidate.code,
                    candidate.weight
                );
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
