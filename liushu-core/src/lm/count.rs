//! 语料统计：一遍数词，一遍数词对。
//!
//! 语料每行 `词序列<TAB>拼音序列`，两部分都用空格分开、逐 token 对齐（第 i 个拼音就是第 i 个
//! 词的读音）。**词级模型只用第一列**：编码到词由词库负责，语言模型负责「词接词」的概率。
//! 第二列仍然要求对齐（不一致说明语料有问题），没有制表符的行按「只有词序列」处理。
//!
//! 统计出来的 [`Counts`] 是训练器与打包器之间的中间表示：词按词面升序编号，词对按
//! `(前词, 后词)` 升序，同一份语料跑两遍得到字节相同的文件。
//!
//! 第二遍只数 id 对：`(u32, u32)` 压进一个 `u64` 当键，省掉把两个词的字符串各存一份。

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::LiushuError;

/// 一个词的统计。
#[derive(Debug)]
pub(crate) struct WordCounts {
    /// 词面。
    pub text: String,

    /// 在语料里出现的次数。
    pub count: u64,
}

/// 训练中间表示。
#[derive(Debug)]
pub(crate) struct Counts {
    /// 全部词的计数之和，一元概率的分母。
    pub total: u64,

    /// 全部词，按词面升序；下标就是词的编号。
    pub words: Vec<WordCounts>,

    /// `(前词, 后词, 次数)`，按 `(前词, 后词)` 升序。
    pub transitions: Vec<(u32, u32, u32)>,
}

/// 把一行拆成空格分隔的 token。
fn tokens(field: &str) -> Vec<Box<str>> {
    field
        .split(' ')
        .filter(|token| !token.is_empty())
        .map(Into::into)
        .collect()
}

/// 逐行读语料，把词序列交给 `visit`。空行、`#` 开头的行跳过。
fn for_each_sentence(
    corpus: &Path,
    mut visit: impl FnMut(&[Box<str>]) -> Result<(), LiushuError>,
) -> Result<(), LiushuError> {
    let file = File::open(corpus)?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let sentence = tokens(fields.next().unwrap_or_default());
        if sentence.is_empty() {
            continue;
        }
        // 有拼音列就要求逐 token 对齐：对不上多半是语料切分或者拼音列错位了。
        if let Some(pinyin) = fields.next() {
            let pinyin = tokens(pinyin);
            if pinyin.len() != sentence.len() {
                return Err(LiushuError::Other(format!(
                    "corpus line {}: {} words but {} pinyins",
                    index + 1,
                    sentence.len(),
                    pinyin.len()
                )));
            }
        }
        visit(&sentence)?;
    }
    Ok(())
}

/// 数词。
fn count_words(corpus: &Path) -> Result<BTreeMap<Box<str>, u64>, LiushuError> {
    let mut words: BTreeMap<Box<str>, u64> = BTreeMap::new();
    for_each_sentence(corpus, |sentence| {
        for word in sentence {
            *words.entry(word.clone()).or_default() += 1;
        }
        Ok(())
    })?;
    Ok(words)
}

/// 跑两遍语料，得到训练中间表示。
pub(crate) fn count(corpus: &Path) -> Result<Counts, LiushuError> {
    let raw = count_words(corpus)?;
    let total: u64 = raw.values().sum();

    // 词面升序就是编号顺序：同一份语料两次训练得到同一份文件。
    let words: Vec<WordCounts> = raw
        .iter()
        .map(|(text, count)| WordCounts {
            text: text.to_string(),
            count: *count,
        })
        .collect();
    let ids: HashMap<&str, u32> = raw
        .keys()
        .enumerate()
        .map(|(index, text)| (text.as_ref(), index as u32))
        .collect();

    let mut transitions: HashMap<u64, u32> = HashMap::new();
    for_each_sentence(corpus, |sentence| {
        let ids: Vec<u32> = sentence
            .iter()
            .filter_map(|word| ids.get(word.as_ref()).copied())
            .collect();
        for pair in ids.windows(2) {
            *transitions
                .entry((u64::from(pair[0]) << 32) | u64::from(pair[1]))
                .or_default() += 1;
        }
        Ok(())
    })?;

    let mut transitions: Vec<(u32, u32, u32)> = transitions
        .into_iter()
        .map(|(key, count)| ((key >> 32) as u32, key as u32, count))
        .collect();
    transitions.sort_unstable();

    Ok(Counts {
        total,
        words,
        transitions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_corpus(name: &str, body: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("liushu-corpus-{name}-{}.tsv", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn id(counts: &Counts, word: &str) -> u32 {
        counts
            .words
            .iter()
            .position(|entry| entry.text == word)
            .unwrap() as u32
    }

    #[test]
    fn counts_words_and_transitions() {
        let corpus = write_corpus("counts", "我 想 去\two xiang qu\n我 想 吃\two xiang chi\n");
        let counts = count(&corpus).unwrap();
        assert_eq!(counts.total, 6);
        let words: Vec<&str> = counts.words.iter().map(|word| word.text.as_str()).collect();
        // 词按 UTF-8 字节序编号。
        assert_eq!(words, ["去", "吃", "想", "我"]);
        assert_eq!(counts.words.iter().map(|word| word.count).sum::<u64>(), 6);
        // (我, 想) 两次，(想, 去) 与 (想, 吃) 各一次；按 (前词, 后词) 升序。
        assert_eq!(
            counts.transitions,
            vec![
                (id(&counts, "想"), id(&counts, "去"), 1),
                (id(&counts, "想"), id(&counts, "吃"), 1),
                (id(&counts, "我"), id(&counts, "想"), 2),
            ]
        );
        std::fs::remove_file(&corpus).unwrap();
    }

    #[test]
    fn a_line_without_pinyin_is_words_only() {
        let corpus = write_corpus("words-only", "我 想 去\n我 想 去\n");
        let counts = count(&corpus).unwrap();
        assert_eq!(counts.total, 6);
        std::fs::remove_file(&corpus).unwrap();
    }

    #[test]
    fn a_misaligned_pinyin_column_is_an_error_with_its_line_number() {
        let corpus = write_corpus("misaligned", "我 想 去\two xiang\n");
        let error = count(&corpus).unwrap_err().to_string();
        assert!(error.contains("line 1"), "{error}");
        std::fs::remove_file(&corpus).unwrap();
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let corpus = write_corpus("comments", "# 注释\n\n我 想 去\two xiang qu\n");
        let counts = count(&corpus).unwrap();
        assert_eq!(counts.total, 3);
        std::fs::remove_file(&corpus).unwrap();
    }
}
