//! 词级语言模型：`P(词 | 前一个词)`。
//!
//! 旧实现是字符级的：每个拼音音节查「哪些字读这个音」，再在字与字之间做维特比。那样模型根本
//! 不知道「词」，也没法给「你好」这类多音节词打分。现在语言模型只回答一个问题——**某个词接在
//! 另一个词后面有多常见**——编码到词由词库负责，解码器把两者相加（见 `engine::sentence`）。
//!
//! ```text
//! WORD 词文本 arena
//! WENT 按词编号升序的 `WordEntry`（8 字节/条）：词在 arena 里的位置 + 一元 log 概率
//! HASH 词面 → 编号的开放寻址哈希索引（`u32` 槽，装载率 ≤ 0.5）
//! OFFS 转移表的 CSR 段偏移（`u32`，长度 = 词数 + 1）
//! TRAN `Transition`（8 字节/条）：前词 `v` 的后继在 `transitions[offsets[v]..offsets[v + 1]]`
//! ```
//!
//! 概率一律 **Q4 定点**（1 log 单位 = 16 个整数步，`i16` 覆盖 ±2048），解码全程整数运算。
//! 平滑是插值：`P(w | v) = λ·c(v,w)/c(v) + (1-λ)·P(w)`。**插值在训练时就做完**，所以解码
//! 只需要查表，不用在按键路径上算 `exp` / `ln`；被剪掉的词对精确地退化成一元概率，查不到时
//! 现场补一个常数偏移即可：
//!
//! ```text
//! 表里有   → 训练时算好的 λ·c(v,w)/c(v) + (1-λ)·P(w)
//! 表里没有 → ln P(w) + ln(1-λ)          （与训练时对 c(v,w)=0 的取值完全一致）
//! ```
//!
//! 词面 → 编号的哈希函数必须跨进程、跨版本稳定（写文件的进程和读文件的进程要算得一样），
//! 所以用固定的 FNV-1a 64 再过一遍 MurmurHash3 的 fmix64 终混：UTF-8 中文全是 0xE4–0xE9
//! 开头的三字节，FNV 的低位分布不好，而开放寻址只用低位选槽。

mod count;

use std::path::Path;

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::error::LiushuError;
use crate::format::{self, Container, Section, Table, Text};

use count::Counts;

/// 容器种类。
const KIND: [u8; 4] = *b"LMOD";

const WORD_TAG: [u8; 4] = *b"WORD";
const WORD_ENTRY_TAG: [u8; 4] = *b"WENT";
const HASH_TAG: [u8; 4] = *b"HASH";
const OFFSET_TAG: [u8; 4] = *b"TOFF";
const TRANSITION_TAG: [u8; 4] = *b"TRAN";

/// 定点精度：1 log 单位（自然对数）用 16 个整数步表示，`i16` 因此覆盖 ±2048。
const Q4: i32 = 16;

/// 插值时二元部分占的权重，其余给一元概率。
const LAMBDA: f64 = 0.8;

/// `ln(1 - λ)` 的定点值，约 -1.6094 × 16。
const LN_ONE_MINUS_LAMBDA: i32 = -26;

/// 一个词对至少出现几次才值得存。出现一次的多半是分词噪声，剪掉能省一半以上的转移表。
const MIN_TRANSITION_COUNT: u32 = 2;

/// 一个前词最多保留多少个后继（按次数取前几名）。
const MAX_TRANSITIONS_PER_WORD: usize = 64;

/// 哈希表里的空槽。
const EMPTY: u32 = u32::MAX;

/// 按 Q4 量化一个 log 概率（非正数），并夹在 `i16` 里。
fn quantize(log_prob: f64) -> i16 {
    let scaled = (log_prob * f64::from(Q4)).round();
    scaled.clamp(f64::from(i16::MIN + 1), 0.0) as i16
}

/// 反量化。解码器全程用定点整数，这个只在测试与对外报告分数时用。
pub fn dequantize(quantized: i32) -> f64 {
    f64::from(quantized) / f64::from(Q4)
}

/// 词表里的一条：词在 arena 里的位置与一元 log 概率。8 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct WordEntry {
    /// 在 `words` 里的字节偏移。
    text_start: u32,

    /// Q4 定点的一元 log 概率。
    log_prob: i16,

    /// 字节长度。
    text_len: u16,
}

/// 一条转移：后词编号与 `ln P(后词 | 前词)`（已插值平滑）。8 字节、无填充，原样落盘。
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

/// 稳定哈希：FNV-1a 64 再走一遍 fmix64。改这个函数会让已有文件的哈希索引全部失效。
pub fn stable_hash(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in key.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    hash ^ (hash >> 33)
}

/// 装 `count` 条要多大的表：2 的幂、至少两倍条数（装载率 ≤ 0.5，线性探测平均一两步）。
fn capacity_for(count: usize) -> usize {
    count.saturating_mul(2).next_power_of_two().max(16)
}

/// 内存语言模型：从语料训练出来时各段是自己的，从文件打开时是映射里的，查询代码不区分。
#[derive(Debug, Default)]
pub struct LanguageModel {
    /// 所有词首尾相接。
    words: Text,

    /// 按词编号升序。
    entries: Table<WordEntry>,

    /// 词面 → 编号的开放寻址索引。
    index: Table<u32>,

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
            index: container.table(HASH_TAG)?,
            offsets: container.table(OFFSET_TAG)?,
            transitions: container.table(TRANSITION_TAG)?,
        };
        model.validate()?;
        Ok(model)
    }

    /// 校验各段互相引用得通：映射里任何越界都要在这里拦住，而不是在查询时 panic。
    fn validate(&self) -> Result<(), LiushuError> {
        let corrupt = |what: &str| LiushuError::Other(format!("language model is corrupt: {what}"));
        for entry in self.entries.iter() {
            let end = entry.text_start as usize + usize::from(entry.text_len);
            if self.words.get(entry.text_start as usize..end).is_none() {
                return Err(corrupt("word points outside the arena"));
            }
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
        // 索引必须能终止线性探测：容量是 2 的幂、正好装了 count 条、还有空槽。
        let count = self.entries.len();
        let filled = self.index.iter().filter(|&&slot| slot != EMPTY).count();
        if !self.index.len().is_power_of_two()
            || self.index.len() <= count
            || filled != count
            || self
                .index
                .iter()
                .any(|&slot| slot != EMPTY && slot as usize >= count)
        {
            return Err(corrupt("hash index does not match the word table"));
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

    /// 词面 → 编号。模型里没有这个词就是 `None`。
    pub fn word_id(&self, word: &str) -> Option<u32> {
        if self.index.is_empty() {
            return None;
        }
        let mask = self.index.len() - 1;
        let mut slot = (stable_hash(word) as usize) & mask;
        loop {
            let id = self.index[slot];
            if id == EMPTY {
                return None;
            }
            if self.word(id) == word {
                return Some(id);
            }
            slot = (slot + 1) & mask;
        }
    }

    /// 一元 log 概率（Q4 定点）。
    pub fn unigram_log_prob(&self, word: &str) -> Option<i32> {
        let id = self.word_id(word)?;
        Some(i32::from(self.entries[id as usize].log_prob))
    }

    /// 转移 log 概率（Q4 定点），`None` 表示这个词对被剪掉或者没统计到。
    pub fn transition_log_prob(&self, previous: u32, word: u32) -> Option<i32> {
        let start = *self.offsets.get(previous as usize)? as usize;
        let end = *self.offsets.get(previous as usize + 1)? as usize;
        let successors = &self.transitions[start..end];
        let index = successors
            .binary_search_by(|transition| transition.word.cmp(&word))
            .ok()?;
        Some(i32::from(successors[index].log_prob))
    }

    /// `ln P(词 | 前词)`（Q4 定点）。模型里没有「词」本身时返回 `None`。
    ///
    /// - 句首（没有前词）或者前词不在模型里：只有一元概率可用，返回 `ln P(词)`。
    /// - 前词在模型里、词对被剪掉：返回与训练时 `c(v,w)=0` 完全一样的插值结果
    ///   `ln P(词) + ln(1-λ)`。
    pub fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<i32> {
        let id = self.word_id(word)?;
        let unigram = i32::from(self.entries[id as usize].log_prob);
        match previous.and_then(|previous| self.word_id(previous)) {
            Some(previous) => Some(
                self.transition_log_prob(previous, id)
                    .unwrap_or_else(|| unigram.saturating_add(LN_ONE_MINUS_LAMBDA)),
            ),
            None => Some(unigram),
        }
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
                    tag: HASH_TAG,
                    bytes: self.index.as_bytes(),
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

/// 语料统计 → 内存模型：词表、哈希索引、一元概率、插值平滑后的 CSR 转移表。
fn from_counts(counts: &Counts) -> LanguageModel {
    let total = counts.total.max(1) as f64;

    let mut words = String::new();
    let mut entries = Vec::with_capacity(counts.words.len());
    for word in &counts.words {
        entries.push(WordEntry {
            text_start: words.len() as u32,
            log_prob: quantize((word.count as f64 / total).ln()),
            text_len: word.text.len() as u16,
        });
        words.push_str(&word.text);
    }

    // 词面 → 编号的开放寻址索引，槽里放编号，键留在 arena 里。
    let mut index = vec![EMPTY; capacity_for(counts.words.len())];
    let mask = index.len() - 1;
    for (id, word) in counts.words.iter().enumerate() {
        let id = u32::try_from(id).expect("hash index holds at most u32::MAX entries");
        let mut slot = (stable_hash(&word.text) as usize) & mask;
        while index[slot] != EMPTY {
            slot = (slot + 1) & mask;
        }
        index[slot] = id;
    }

    // 转移：按前词分组成 CSR。段内的分母是**剪枝前**的真实总数，剪枝才不会把概率抬起来。
    let unigram: Vec<f64> = counts
        .words
        .iter()
        .map(|word| (word.count as f64 / total).ln())
        .collect();
    let mut offsets = vec![0u32];
    let mut transitions = Vec::new();
    let mut cursor = 0;
    for previous in 0..counts.words.len() {
        let mut pair_total = 0u64;
        let mut group: Vec<(u32, u32)> = Vec::new();
        while cursor < counts.transitions.len() && counts.transitions[cursor].0 as usize == previous
        {
            let (_, word, count) = counts.transitions[cursor];
            pair_total += u64::from(count);
            group.push((word, count));
            cursor += 1;
        }
        group.sort_unstable_by_key(|&(word, count)| (std::cmp::Reverse(count), word));
        group.truncate(MAX_TRANSITIONS_PER_WORD);
        group.retain(|&(_, count)| count >= MIN_TRANSITION_COUNT);
        group.sort_unstable_by_key(|&(word, _)| word);
        for (word, count) in group {
            // 插值在训练时算完：λ·c(v,w)/c(v) + (1-λ)·P(w)。
            let bigram = count as f64 / pair_total.max(1) as f64;
            let interpolated = LAMBDA * bigram + (1.0 - LAMBDA) * unigram[word as usize].exp();
            transitions.push(Transition {
                word,
                log_prob: quantize(interpolated.ln()),
                reserved: 0,
            });
        }
        offsets.push(transitions.len() as u32);
    }

    LanguageModel {
        words: Text::Owned(words),
        entries: Table::Owned(entries),
        index: Table::Owned(index),
        offsets: Table::Owned(offsets),
        transitions: Table::Owned(transitions),
    }
}

/// 从语料训练一个词级语言模型并写成文件。
pub fn train(corpus: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), LiushuError> {
    let counts = count::count(corpus.as_ref())?;
    if counts.words.is_empty() {
        return Err(LiushuError::Other("corpus has no words".to_owned()));
    }
    from_counts(&counts).write(output.as_ref())
}

#[cfg(test)]
mod tests;
