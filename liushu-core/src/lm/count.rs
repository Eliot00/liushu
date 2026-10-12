//! 语料统计：一遍数一元 / 发射，一遍数二元。
//!
//! 语料每行 `句子<TAB>拼音`，两部分都用空格分开、逐 token 对齐（第 i 个拼音就是第 i 个
//! 词的读音）。统计出来的 [`Counts`] 是训练器与打包器之间的中间表示：词按文本升序编号，
//! 二元组按 `(前词, 后词)` 升序，同一份语料跑两遍得到字节相同的文件。
//!
//! 第二遍只数 id 对：`(u32, u32)` 压进一个 `u64` 当键，省掉把两个词的字符串各存一份。

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::LiushuError;

/// 一个词的一元与发射计数。发射按拼音升序。
#[derive(Debug)]
pub(crate) struct WordCounts {
    /// 词面。
    pub text: String,

    /// 作为句子第一个词出现的次数。
    pub init: u64,

    /// `(拼音, 次数)`，按拼音升序。
    pub emissions: Vec<(String, u64)>,
}

/// 训练中间表示。
#[derive(Debug)]
pub(crate) struct Counts {
    /// 句子总数，一元概率的分母。
    pub sentences: u64,

    /// 全部词，按词面升序；下标就是词的编号。
    pub words: Vec<WordCounts>,

    /// `(前词, 后词, 次数)`，按 `(前词, 后词)` 升序。
    pub transitions: Vec<(u32, u32, u32)>,
}

/// 逐行读语料，把对齐好的句子与拼音交给 `visit`。空行、`#` 开头的行跳过。
fn for_each_sentence(
    corpus: &Path,
    mut visit: impl FnMut(&[Box<str>], &[Box<str>]) -> Result<(), LiushuError>,
) -> Result<(), LiushuError> {
    let file = File::open(corpus)?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(sentence), Some(pinyin)) = (fields.next(), fields.next()) else {
            return Err(LiushuError::Other(format!(
                "corpus line {}: expected `sentence<TAB>pinyin`",
                index + 1
            )));
        };
        let split = |field: &str| -> Vec<Box<str>> {
            field
                .split(' ')
                .filter(|token| !token.is_empty())
                .map(Into::into)
                .collect()
        };
        let sentence = split(sentence);
        if sentence.is_empty() {
            continue;
        }
        visit(&sentence, &split(pinyin))?;
    }
    Ok(())
}

/// 一个词的一元计数与「拼音 → 次数」。
type RawWord = (u64, BTreeMap<Box<str>, u64>);

/// 词面 → 一元与发射计数。
type RawWords = BTreeMap<Box<str>, RawWord>;

/// 数一元与发射。
fn count_words(corpus: &Path) -> Result<(u64, RawWords), LiushuError> {
    let mut sentences = 0u64;
    let mut words: RawWords = BTreeMap::new();
    for_each_sentence(corpus, |sentence, pinyin| {
        sentences += 1;
        words.entry(sentence[0].clone()).or_default().0 += 1;
        for (word, pinyin) in sentence.iter().zip(pinyin) {
            *words
                .entry(word.clone())
                .or_default()
                .1
                .entry(pinyin.clone())
                .or_default() += 1;
        }
        Ok(())
    })?;
    Ok((sentences, words))
}

/// 跑两遍语料，得到训练中间表示。
pub(crate) fn count(corpus: &Path) -> Result<Counts, LiushuError> {
    let (sentences, raw) = count_words(corpus)?;

    // 词面升序就是编号顺序：同一份语料两次训练得到同一份文件。
    let words: Vec<WordCounts> = raw
        .iter()
        .map(|(text, (init, emissions))| WordCounts {
            text: text.to_string(),
            init: *init,
            emissions: emissions
                .iter()
                .map(|(pinyin, count)| (pinyin.to_string(), *count))
                .collect(),
        })
        .collect();
    let ids: HashMap<&str, u32> = raw
        .keys()
        .enumerate()
        .map(|(index, text)| (text.as_ref(), index as u32))
        .collect();

    let mut transitions: HashMap<u64, u32> = HashMap::new();
    for_each_sentence(corpus, |sentence, _| {
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
        sentences,
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

    #[test]
    fn counts_words_emissions_and_transitions() {
        let corpus = write_corpus("counts", "我 想 去\two xiang qu\n我 想 吃\two xiang chi\n");
        let counts = count(&corpus).unwrap();
        assert_eq!(counts.sentences, 2);
        let words: Vec<&str> = counts.words.iter().map(|word| word.text.as_str()).collect();
        // 词按 UTF-8 字节序编号，不是按拼音顺序。
        assert_eq!(words, ["去", "吃", "想", "我"]);
        let wo = counts.words.iter().find(|word| word.text == "我").unwrap();
        assert_eq!(wo.init, 2);
        assert_eq!(wo.emissions, [("wo".to_string(), 2)]);
        // (我, 想) 两次，(想, 吃) 与 (想, 去) 各一次；按 (前词, 后词) 升序。
        assert_eq!(
            counts.transitions,
            vec![
                (ids(&counts, "想"), ids(&counts, "去"), 1),
                (ids(&counts, "想"), ids(&counts, "吃"), 1),
                (ids(&counts, "我"), ids(&counts, "想"), 2),
            ]
        );
        std::fs::remove_file(&corpus).unwrap();
    }

    fn ids(counts: &Counts, word: &str) -> u32 {
        counts
            .words
            .iter()
            .position(|entry| entry.text == word)
            .unwrap() as u32
    }

    #[test]
    fn a_broken_line_is_an_error_with_its_line_number() {
        let corpus = write_corpus("broken", "我 想 去\n");
        let error = count(&corpus).unwrap_err().to_string();
        assert!(error.contains("line 1"), "{error}");
        std::fs::remove_file(&corpus).unwrap();
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let corpus = write_corpus("comments", "# 注释\n\n我 想 去\two xiang qu\n");
        let counts = count(&corpus).unwrap();
        assert_eq!(counts.sentences, 1);
        std::fs::remove_file(&corpus).unwrap();
    }
}
