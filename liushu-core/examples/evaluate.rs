//! 开发用评测：拿一批「词序列 + 拼音」的句子，测整句解码的还原率与每句耗时。
//!
//! ```text
//! cargo run --release -p liushu-core --example evaluate -- dict.bin model.bin test.tsv [limit]
//! ```
//!
//! `test.tsv` 每行 `词序列<TAB>拼音序列`（与训练语料同一个格式，拼音按 token 对齐）。评测时把
//! 拼音连起来当输入——这正是输入法里用户敲的那个字符串。报告三件事：整句完全还原的比例、
//! 字符级的 LCS 还原率（部分对也计分）、以及每句解码的耗时。

use std::time::Instant;

use liushu_core::engine::Engine;

/// 最长公共子序列长度，用来给「没完全还原」的句子一点部分分。
fn lcs(reference: &[char], decoded: &[char]) -> usize {
    let mut previous = vec![0usize; decoded.len() + 1];
    let mut current = vec![0usize; decoded.len() + 1];
    for left in reference {
        for (index, right) in decoded.iter().enumerate() {
            current[index + 1] = if left == right {
                previous[index] + 1
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[decoded.len()]
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (Some(dict_path), Some(model_path), Some(test_path)) =
        (arguments.first(), arguments.get(1), arguments.get(2))
    else {
        eprintln!("usage: evaluate <dict.bin> <model.bin> <test.tsv> [limit]");
        return;
    };
    let limit: usize = arguments
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);

    let started = Instant::now();
    let engine = Engine::with_model(dict_path, model_path).expect("load engine");
    println!("引擎加载 {:.1?}", started.elapsed());

    let body = std::fs::read_to_string(test_path).expect("read test file");
    let mut exact = 0usize;
    let mut matched = 0usize;
    let mut total = 0usize;
    let mut sentences = 0usize;
    let mut elapsed = std::time::Duration::ZERO;
    for line in body.lines().take(limit) {
        let mut fields = line.split('\t');
        let (Some(reference), Some(pinyin)) = (fields.next(), fields.next()) else {
            continue;
        };
        let reference: String = reference.split(' ').collect();
        let code: String = pinyin.split(' ').collect();
        sentences += 1;

        let started = Instant::now();
        let decoded = engine.sentences(&code, 1);
        elapsed += started.elapsed();

        let decoded = decoded
            .first()
            .map(|sentence| sentence.text.clone())
            .unwrap_or_default();
        let reference_chars: Vec<char> = reference.chars().collect();
        let decoded_chars: Vec<char> = decoded.chars().collect();
        matched += lcs(&reference_chars, &decoded_chars);
        total += reference_chars.len();
        exact += usize::from(decoded == reference);
    }

    println!(
        "句子 {} 条：整句完全还原 {} 条（{:.1}%），字符级还原 {:.1}%（{} / {}）",
        sentences,
        exact,
        100.0 * exact as f64 / sentences.max(1) as f64,
        100.0 * matched as f64 / total.max(1) as f64,
        matched,
        total
    );
    println!("每句解码平均 {:.1?}", elapsed / sentences.max(1) as u32);
}
