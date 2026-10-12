//! 开发用计时：打开词库 / 语言模型，报告文件大小与打开耗时。
//!
//! ```text
//! cargo run --release -p liushu-core --example timing -- dict.bin [model.bin] [pinyin]
//! ```
//!
//! 只给词库路径就只测词库；再给模型路径就一起测整句解码（第三个参数是拼音，默认 `nihao`）。

use std::path::PathBuf;
use std::time::Instant;

use liushu_core::{
    engine::{Engine, InputMethodEngine},
    lm::LanguageModel,
};

fn size(path: &std::path::Path) -> String {
    let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn main() {
    let arguments: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let Some(dictionary_path) = arguments.first() else {
        eprintln!("usage: timing <dict.bin> [model.bin] [pinyin]");
        return;
    };
    let model_path = arguments.get(1).filter(|path| path.exists());
    let code = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "nihao".to_owned());

    let started = Instant::now();
    let engine = match model_path {
        Some(model_path) => Engine::with_model(dictionary_path, model_path),
        None => Engine::new(dictionary_path),
    };
    let engine = match engine {
        Ok(engine) => engine,
        Err(error) => {
            println!("{}: {error}", dictionary_path.display());
            return;
        }
    };
    let open = started.elapsed();
    println!(
        "{}: {} 引擎打开 {:.1?}",
        dictionary_path.display(),
        size(dictionary_path),
        open
    );

    let started = Instant::now();
    let candidates = engine.search(&code).map(|list| list.len()).unwrap_or(0);
    println!(
        "  search({code:?}) {:.1?}，命中 {candidates} 条候选",
        started.elapsed()
    );

    if let Some(model_path) = model_path {
        let started = Instant::now();
        let model = LanguageModel::open(model_path).unwrap();
        println!(
            "{}: {} 单独打开 {:.1?}（词 {} 条、转移 {} 条）",
            model_path.display(),
            size(model_path),
            started.elapsed(),
            model.word_count(),
            model.transition_count()
        );
        let started = Instant::now();
        let sentences = engine.sentences(&code, 1);
        let first = started.elapsed();
        // 第一次调用要把词库和模型的页从磁盘带进来，所以再跑若干次取个热态的平均值。
        const ROUNDS: u32 = 20;
        let started = Instant::now();
        for _ in 0..ROUNDS {
            let _ = engine.sentences(&code, 1);
        }
        println!(
            "  sentences({code:?}) 首次 {:.1?}、热态平均 {:.1?}，{}",
            first,
            started.elapsed() / ROUNDS,
            match sentences.first() {
                Some(sentence) => format!("{:?}（{:.2}）", sentence.text, sentence.score),
                None => "没有整句候选".to_owned(),
            }
        );
    }
}
