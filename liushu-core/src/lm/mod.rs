//! 语言模型：把一串拼音音节解成最可能的词序列。
//!
//! 旧实现把三元表（一元 / 发射 / 转移）全塞进 `redb`，值一律 `f64`：一个 `(词, 拼音)`
//! 键是两份字符串 + B-tree 页头 + 8 字节浮点，几十万条就是几百 MB，而且每次按键都要
//! 走一遍 KV 查找。现在改成**文件布局就是内存布局**：
//!
//! ```text
//! WORD 词文本 arena
//! WENT 按键的编号升序排好的 `WordEntry`（8 字节/条）
//! PY   拼音 arena
//! PYIX 按拼音字节序升序排好的 `PyIndex`（16 字节/条）
//! EMIT 与 PYIX 同序的 `Emission`（8 字节/条）：该拼音能发射哪些词、各自多大概率
//! INIT 第一项是句子开头的 `InitEntry`（8 字节/条），按词编号升序
//! TOFF 转移表的 CSR 段偏移（`u32`，长度 = 词数 + 1）
//! TRAN `Transition`（8 字节/条）：前词 `v` 的后继在 `successors[offsets[v]..offsets[v + 1]]`
//! ```
//!
//! 概率一律 **Q4 定点**（1 log 单位 = 16 个整数步，`i16` 覆盖 ±2048）——比 `f64` 省四倍，
//! 而且解码全程整数运算，跨平台没有浮点舍入漂移。转移表按前词分组成 CSR，段内按后词编号
//! 升序：整句转换每一步都要查它，一段通常只有几条，落在一两个缓存行里；平铺一张大表二分
//! 一次要跳二十来个缓存行。低频二元组按次数剪掉（见 [`MIN_TRANSITION_COUNT`]）。

mod count;

use std::collections::BTreeMap;
use std::path::Path;

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::error::LiushuError;
use crate::format::{self, Container, Section, Table, Text};

use count::Counts;

/// 容器种类。
const KIND: [u8; 4] = *b"LMOD";

const WORD_TAG: [u8; 4] = *b"WORD";
const WORD_ENTRY_TAG: [u8; 4] = *b"WENT";
const PINYIN_TAG: [u8; 4] = *b"PY\x00\x00";
const PINYIN_INDEX_TAG: [u8; 4] = *b"PYIX";
const EMISSION_TAG: [u8; 4] = *b"EMIT";
const INIT_TAG: [u8; 4] = *b"INIT";
const OFFSET_TAG: [u8; 4] = *b"TOFF";
const TRANSITION_TAG: [u8; 4] = *b"TRAN";

/// 定点精度：1 log 单位（自然对数）用 16 个整数步表示，`i16` 因此覆盖 ±2048。
const Q4: i32 = 16;

/// 一个二元组至少出现几次才值得存。出现一次的多半是分词噪声，剪掉能省一半以上的转移表。
const MIN_TRANSITION_COUNT: u32 = 2;

/// 一个前词最多保留多少个后继（按次数取前几名）。
const MAX_TRANSITIONS_PER_WORD: usize = 64;

/// 表里查不到的一元 / 转移概率的兜底：`ln(6e-6) ≈ -12`。
///
/// 旧实现用 `-3.14e100` 代表「没这条边」，任何含未知二元组的路径都会被一票否决，
/// 于是解码器被迫在「词频高但搭配没见过」和「搭配见过但词很生僻」之间选后者。
/// 这里给一个很低但有限的分数：没见过就是很不像，但不是不可能。
const FLOOR_LOG_PROB: f64 = -12.0;

/// 兜底分数对应的定点值。
const FLOOR: i32 = FLOOR_LOG_PROB as i32 * Q4;

/// 按 Q4 量化一个 log 概率（非正数），并夹在 `i16` 里。
fn quantize(log_prob: f64) -> i16 {
    let scaled = (log_prob * f64::from(Q4)).round();
    scaled.clamp(f64::from(i16::MIN + 1), 0.0) as i16
}

/// 反量化（测试与调用方读分数用）。
pub fn dequantize(quantized: i16) -> f64 {
    f64::from(quantized) / f64::from(Q4)
}

/// 词表里的一条：词在 arena 里的位置。8 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct WordEntry {
    /// 在 `words` 里的字节偏移。
    text_start: u32,

    /// 字节长度。
    text_len: u16,

    /// 对齐用，全零。
    reserved: u16,
}

/// 一个拼音及其发射段。16 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct PyIndex {
    /// 在 `pinyins` 里的字节偏移。
    py_start: u32,

    /// 发射在 `emissions` 里的起始下标。
    first_emission: u32,

    /// 发射条数。
    emission_count: u32,

    /// 字节长度。
    py_len: u16,

    /// 对齐用，全零。
    reserved: u16,
}

/// 一条发射：词编号与 `ln P(词 | 拼音)`。8 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct Emission {
    /// 词编号。
    pub word: u32,

    /// Q4 定点的 log 概率。
    pub log_prob: i16,

    /// 对齐用，全零。
    reserved: u16,
}

/// 一条句首概率。8 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct InitEntry {
    /// 词编号。
    word: u32,

    /// Q4 定点的 log 概率。
    log_prob: i16,

    /// 对齐用，全零。
    reserved: u16,
}

/// 一条转移：后词编号与 `ln P(后词 | 前词)`。8 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct Transition {
    /// 后词编号。
    word: u32,

    /// Q4 定点的 log 概率。
    log_prob: i16,

    /// 对齐用，全零。
    reserved: u16,
}

/// 内存语言模型：从语料训练出来时各段是自己的，从文件打开时是映射里的，解码代码不区分。
#[derive(Debug, Default)]
pub struct LanguageModel {
    /// 所有词首尾相接。
    words: Text,

    /// 按词编号升序。
    entries: Table<WordEntry>,

    /// 所有拼音首尾相接。
    pinyins: Text,

    /// 按拼音的 UTF-8 字节序升序。
    pinyin_index: Table<PyIndex>,

    /// 与 `pinyin_index` 同序。
    emissions: Table<Emission>,

    /// 按词编号升序。
    init: Table<InitEntry>,

    /// CSR 段偏移，长度 = 词数 + 1。
    offsets: Table<u32>,

    /// CSR 后继，段内按后词编号升序。
    transitions: Table<Transition>,
}

impl LanguageModel {
    /// 从文件打开（`mmap`，不反序列化）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LiushuError> {
        let container = Container::open(path, KIND)?;
        let model = Self {
            words: container.text(WORD_TAG)?,
            entries: container.table(WORD_ENTRY_TAG)?,
            pinyins: container.text(PINYIN_TAG)?,
            pinyin_index: container.table(PINYIN_INDEX_TAG)?,
            emissions: container.table(EMISSION_TAG)?,
            init: container.table(INIT_TAG)?,
            offsets: container.table(OFFSET_TAG)?,
            transitions: container.table(TRANSITION_TAG)?,
        };
        model.validate()?;
        Ok(model)
    }

    /// 校验各段互相引用得通：映射里任何越界都要在这里拦住，而不是在解码时 panic。
    fn validate(&self) -> Result<(), LiushuError> {
        let corrupt = |what: &str| LiushuError::Other(format!("language model is corrupt: {what}"));
        for entry in self.entries.iter() {
            let end = entry.text_start as usize + usize::from(entry.text_len);
            if self.words.get(entry.text_start as usize..end).is_none() {
                return Err(corrupt("word points outside the arena"));
            }
        }
        for index in self.pinyin_index.iter() {
            let py_end = index.py_start as usize + usize::from(index.py_len);
            if self.pinyins.get(index.py_start as usize..py_end).is_none() {
                return Err(corrupt("pinyin points outside the arena"));
            }
            let emission_end = index.first_emission as usize + index.emission_count as usize;
            if emission_end > self.emissions.len() {
                return Err(corrupt("pinyin points outside the emission table"));
            }
            if self.emissions[index.first_emission as usize..emission_end]
                .iter()
                .any(|emission| emission.word as usize >= self.entries.len())
            {
                return Err(corrupt("emission points outside the word table"));
            }
        }
        if self
            .init
            .iter()
            .any(|entry| entry.word as usize >= self.entries.len())
        {
            return Err(corrupt("init points outside the word table"));
        }
        if self.offsets.len() != self.entries.len() + 1
            || self.offsets.windows(2).any(|w| w[0] > w[1])
            || self
                .offsets
                .last()
                .is_some_and(|&end| end as usize != self.transitions.len())
        {
            return Err(corrupt("CSR offsets are inconsistent"));
        }
        if self
            .transitions
            .iter()
            .any(|transition| transition.word as usize >= self.entries.len())
        {
            return Err(corrupt("transition points outside the word table"));
        }
        Ok(())
    }

    /// 词编号个数。
    pub fn word_count(&self) -> usize {
        self.entries.len()
    }

    /// 转移条数。
    pub fn transition_count(&self) -> usize {
        self.transitions.len()
    }

    /// 词编号对应的词。
    pub fn word(&self, id: u32) -> &str {
        let entry = self.entries[id as usize];
        let start = entry.text_start as usize;
        &self.words[start..start + usize::from(entry.text_len)]
    }

    /// 拼音对应的发射段；这个拼音没在模型里出现过时返回 `None`。
    fn emissions_for(&self, pinyin: &str) -> Option<&[Emission]> {
        let index = self
            .pinyin_index
            .binary_search_by(|entry| self.pinyin(*entry).cmp(pinyin))
            .ok()?;
        let entry = self.pinyin_index[index];
        let start = entry.first_emission as usize;
        Some(&self.emissions[start..start + entry.emission_count as usize])
    }

    fn pinyin(&self, index: PyIndex) -> &str {
        let start = index.py_start as usize;
        &self.pinyins[start..start + usize::from(index.py_len)]
    }

    /// 句首概率（Q4 定点），没统计到就是 `None`。
    fn init_log_prob(&self, word: u32) -> Option<i32> {
        let index = self
            .init
            .binary_search_by(|entry| entry.word.cmp(&word))
            .ok()?;
        Some(i32::from(self.init[index].log_prob))
    }

    /// 转移概率（Q4 定点），剪掉或没见过就是 `None`。
    fn transition_log_prob(&self, previous: u32, word: u32) -> Option<i32> {
        let start = *self.offsets.get(previous as usize)? as usize;
        let end = *self.offsets.get(previous as usize + 1)? as usize;
        let successors = &self.transitions[start..end];
        let index = successors
            .binary_search_by(|transition| transition.word.cmp(&word))
            .ok()?;
        Some(i32::from(successors[index].log_prob))
    }

    /// 把一串拼音解成最可能的词序列：每个位置只考虑该拼音能发射的词，维特比取最大。
    ///
    /// 与旧实现相比有两点变化：词表按编号有序，同分时取编号小的（旧实现遍历 `HashMap`，
    /// 同分结果不稳定）；某个拼音完全没候选时报错，而不是返回半截空串。
    pub fn sentence(&self, pinyins: &[String]) -> Result<String, LiushuError> {
        if pinyins.is_empty() {
            return Ok(String::new());
        }

        let candidates: Vec<&[Emission]> = pinyins
            .iter()
            .map(|pinyin| {
                self.emissions_for(pinyin).ok_or_else(|| {
                    LiushuError::Other(format!("no candidate word reads as {pinyin:?}"))
                })
            })
            .collect::<Result<_, _>>()?;

        let mut scores: Vec<Vec<i32>> = Vec::with_capacity(candidates.len());
        let mut back: Vec<Vec<u32>> = Vec::with_capacity(candidates.len());
        for (position, emissions) in candidates.iter().enumerate() {
            let mut position_scores = Vec::with_capacity(emissions.len());
            let mut position_back = Vec::with_capacity(emissions.len());
            if position == 0 {
                for emission in *emissions {
                    let init = self.init_log_prob(emission.word).unwrap_or(FLOOR);
                    position_scores.push(init.saturating_add(i32::from(emission.log_prob)));
                    position_back.push(0);
                }
            } else {
                let previous = &scores[position - 1];
                for emission in *emissions {
                    let mut best = i32::MIN;
                    let mut best_index = 0u32;
                    for (index, previous_emission) in candidates[position - 1].iter().enumerate() {
                        let transition = self
                            .transition_log_prob(previous_emission.word, emission.word)
                            .unwrap_or(FLOOR);
                        let score = previous[index].saturating_add(transition);
                        if score > best {
                            best = score;
                            best_index = index as u32;
                        }
                    }
                    position_scores.push(best.saturating_add(i32::from(emission.log_prob)));
                    position_back.push(best_index);
                }
            }
            scores.push(position_scores);
            back.push(position_back);
        }

        // 末位置取最高分，再顺着回指把整条路径拼回来。
        let last = scores.last().expect("pinyin sequence is not empty");
        let mut best = 0usize;
        for (index, score) in last.iter().enumerate() {
            if *score > last[best] {
                best = index;
            }
        }
        let mut words: Vec<&str> = vec![""; pinyins.len()];
        let mut index = best;
        for position in (0..pinyins.len()).rev() {
            words[position] = self.word(candidates[position][index].word);
            if position > 0 {
                index = back[position][index] as usize;
            }
        }
        Ok(words.concat())
    }

    /// 把各段写成容器文件（先写 `.tmp` 再改名）。
    fn write(&self, path: impl AsRef<Path>) -> Result<(), LiushuError> {
        format::write(
            path.as_ref(),
            KIND,
            &[
                Section {
                    tag: WORD_TAG,
                    bytes: self.words.as_bytes(),
                },
                Section {
                    tag: WORD_ENTRY_TAG,
                    bytes: self.entries.as_bytes(),
                },
                Section {
                    tag: PINYIN_TAG,
                    bytes: self.pinyins.as_bytes(),
                },
                Section {
                    tag: PINYIN_INDEX_TAG,
                    bytes: self.pinyin_index.as_bytes(),
                },
                Section {
                    tag: EMISSION_TAG,
                    bytes: self.emissions.as_bytes(),
                },
                Section {
                    tag: INIT_TAG,
                    bytes: self.init.as_bytes(),
                },
                Section {
                    tag: OFFSET_TAG,
                    bytes: self.offsets.as_bytes(),
                },
                Section {
                    tag: TRANSITION_TAG,
                    bytes: self.transitions.as_bytes(),
                },
            ],
        )
    }
}

/// 语料统计 → 内存模型。分词表、发射表、CSR 转移表都在这里成型。
fn from_counts(counts: &Counts) -> LanguageModel {
    let word_count = counts.words.len();

    // 词文本 arena 与词表。
    let mut words = String::new();
    let mut entries = Vec::with_capacity(word_count);
    for word in &counts.words {
        entries.push(WordEntry {
            text_start: words.len() as u32,
            text_len: word.text.len() as u16,
            reserved: 0,
        });
        words.push_str(&word.text);
    }

    // 发射：按拼音分组，组内按次数降序（同次数取编号小的），概率按该词的全部发射次数归一。
    let mut word_totals = vec![0u64; word_count];
    let mut by_pinyin: BTreeMap<&str, Vec<(u32, u64)>> = BTreeMap::new();
    for (id, word) in counts.words.iter().enumerate() {
        word_totals[id] = word.emissions.iter().map(|(_, count)| count).sum();
        for (pinyin, count) in &word.emissions {
            by_pinyin
                .entry(pinyin.as_str())
                .or_default()
                .push((id as u32, *count));
        }
    }

    let mut pinyins = String::new();
    let mut pinyin_index = Vec::with_capacity(by_pinyin.len());
    let mut emissions = Vec::new();
    for (pinyin, group) in by_pinyin {
        let mut group = group;
        group.sort_unstable_by_key(|&(word, count)| (std::cmp::Reverse(count), word));
        let first_emission = emissions.len() as u32;
        for (word, count) in group {
            let total = word_totals[word as usize].max(1);
            emissions.push(Emission {
                word,
                log_prob: quantize((count as f64 / total as f64).ln()),
                reserved: 0,
            });
        }
        pinyin_index.push(PyIndex {
            py_start: pinyins.len() as u32,
            first_emission,
            emission_count: emissions.len() as u32 - first_emission,
            py_len: pinyin.len() as u16,
            reserved: 0,
        });
        pinyins.push_str(pinyin);
    }

    // 句首概率：分母是句子数。
    let sentences = counts.sentences.max(1) as f64;
    let init: Vec<InitEntry> = counts
        .words
        .iter()
        .enumerate()
        .filter(|(_, word)| word.init > 0)
        .map(|(id, word)| InitEntry {
            word: id as u32,
            log_prob: quantize((word.init as f64 / sentences).ln()),
            reserved: 0,
        })
        .collect();

    // 转移：按前词分组成 CSR。段内的分母是**剪枝前**的真实总数，剪枝才不会把概率抬起来。
    let mut offsets = vec![0u32];
    let mut transitions = Vec::new();
    let mut index = 0;
    for previous in 0..word_count {
        let mut total = 0u64;
        let mut group: Vec<(u32, u32)> = Vec::new();
        while index < counts.transitions.len() && counts.transitions[index].0 as usize == previous {
            let (_, word, count) = counts.transitions[index];
            total += u64::from(count);
            group.push((word, count));
            index += 1;
        }
        group.sort_unstable_by_key(|&(word, count)| (std::cmp::Reverse(count), word));
        group.truncate(MAX_TRANSITIONS_PER_WORD);
        group.retain(|&(_, count)| count >= MIN_TRANSITION_COUNT);
        group.sort_unstable_by_key(|&(word, _)| word);
        for (word, count) in group {
            transitions.push(Transition {
                word,
                log_prob: quantize((f64::from(count) / total.max(1) as f64).ln()),
                reserved: 0,
            });
        }
        offsets.push(transitions.len() as u32);
    }

    LanguageModel {
        words: Text::Owned(words),
        entries: Table::Owned(entries),
        pinyins: Text::Owned(pinyins),
        pinyin_index: Table::Owned(pinyin_index),
        emissions: Table::Owned(emissions),
        init: Table::Owned(init),
        offsets: Table::Owned(offsets),
        transitions: Table::Owned(transitions),
    }
}

/// 从语料训练一个语言模型并写成文件。
pub fn train(corpus: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), LiushuError> {
    let counts = count::count(corpus.as_ref())?;
    from_counts(&counts).write(output.as_ref())
}

/// 打开模型、解一句。
pub fn pinyin_to_sentence(
    pinyins: &[String],
    model_path: impl AsRef<Path>,
) -> Result<String, LiushuError> {
    LanguageModel::open(model_path)?.sentence(pinyins)
}

#[cfg(test)]
mod tests;
