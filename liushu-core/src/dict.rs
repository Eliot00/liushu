//! 拼音词库：编码 → 候选词。
//!
//! 旧实现把 `StringPatriciaMap<Vec<DictItem>>` 直接 `bincode` 落盘：每个候选是
//! `Box<str>` + `Option<Box<str>>`（至少两次堆分配、二十多字节头），每个编码还要一个
//! `Vec` 头，几十万条就是几百 MB，加载还得整份反序列化。现在改成**文件布局就是内存布局**：
//!
//! ```text
//! TEXT 文本 arena：候选词，紧跟其后的就是它的注释（注释长度 0 表示没有注释）
//! CODE 编码 arena（每个编码只存一份）
//! KEYI 按键的 UTF-8 字节序升序排好的 `KeyIndex`（16 字节/条）
//! SLOT 与 KEYI 同序的 `Slot`（12 字节/条），同一个编码下按词频降序
//! ```
//!
//! 没有 trie 指针：键排好序之后「以某前缀开头的编码」永远是连续区间，前缀查询就是两次
//! 二分；候选词与注释只存 `(offset, len)`，不再有 per-item 堆分配、也不必为「没有注释」
//! 的候选留一个偏移字段。打开文件 = `mmap` + 校验边界。

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::error::LiushuError;
use crate::format::{self, Container, Section, Table, Text};

/// 容器种类。
const KIND: [u8; 4] = *b"DICT";

const TEXT_TAG: [u8; 4] = *b"TEXT";
const CODE_TAG: [u8; 4] = *b"CODE";
const KEY_TAG: [u8; 4] = *b"KEYI";
const SLOT_TAG: [u8; 4] = *b"SLOT";

/// 一个候选。
///
/// 编码不在这里：它是候选所在的 [`KeyIndex`] 的键，存一份就够。
#[derive(Debug, Deserialize)]
pub struct DictItem {
    /// 候选词。
    pub text: Box<str>,

    /// 词频，越大越靠前。
    pub weight: u32,

    /// 候选旁的注释。
    pub comment: Option<Box<str>>,
}

/// 一个拼音编码（如 `nihao`）及其下的候选范围。16 字节、无填充，原样落盘。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct KeyIndex {
    /// 编码在 `codes` 里的字节偏移。
    code_start: u32,

    /// 候选在 `slots` 里的起始下标。
    first_slot: u32,

    /// 候选个数（同音词）。
    slot_count: u32,

    /// 编码字节长度。
    code_len: u16,

    /// 对齐用，全零。
    reserved: u16,
}

/// 一条候选。12 字节、无填充，原样落盘。
///
/// 注释紧跟在候选词后面（同一个 arena），所以只要一个起始偏移与两个长度；
/// 注释长度 0 就是没有注释——空字符串注释与没有注释在这里不加区分。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
pub struct Slot {
    /// 候选词在 `texts` 里的字节偏移。
    text_start: u32,

    /// 词频。
    weight: u32,

    /// 候选词字节长度。
    text_len: u16,

    /// 紧跟其后的注释字节长度；0 表示没有注释。
    comment_len: u16,
}

impl Slot {
    /// 词频。
    pub fn weight(&self) -> u32 {
        self.weight
    }
}

/// 输入 TSV 的一行。这里带着编码，因为还没键进索引。
#[derive(Debug, Deserialize)]
struct DictRow {
    text: Box<str>,
    code: Box<str>,
    weight: u32,
    comment: Option<Box<str>>,
}

/// 内存词库：从 TSV 建出来时四段数据是自己的，从文件打开时是映射里的，查询代码不区分。
#[derive(Debug, Default)]
pub struct Dictionary {
    /// 所有候选词及其注释首尾相接。
    texts: Text,

    /// 所有编码首尾相接（同音词共用一个编码）。
    codes: Text,

    /// 按编码的 UTF-8 字节序升序。
    keys: Table<KeyIndex>,

    /// 与 `keys` 同序；同一个编码下按词频降序。
    slots: Table<Slot>,
}

impl Dictionary {
    /// 从文件打开（`mmap`，不反序列化）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LiushuError> {
        let container = Container::open(path, KIND)?;
        let dictionary = Self {
            texts: container.text(TEXT_TAG)?,
            codes: container.text(CODE_TAG)?,
            keys: container.table(KEY_TAG)?,
            slots: container.table(SLOT_TAG)?,
        };
        dictionary.validate()?;
        Ok(dictionary)
    }

    /// 校验各段互相引用得通：映射里任何一个偏移越界都要在这里拦住，而不是在查询时 panic。
    fn validate(&self) -> Result<(), LiushuError> {
        let corrupt = |what: &str| LiushuError::Other(format!("dictionary is corrupt: {what}"));
        for key in self.keys.iter() {
            let code_end = key.code_start as usize + usize::from(key.code_len);
            if self.codes.get(key.code_start as usize..code_end).is_none() {
                return Err(corrupt("key points outside the code arena"));
            }
            let slot_end = key.first_slot as usize + key.slot_count as usize;
            if slot_end > self.slots.len() {
                return Err(corrupt("key points outside the slot table"));
            }
        }
        for slot in self.slots.iter() {
            let end = slot.text_start as usize
                + usize::from(slot.text_len)
                + usize::from(slot.comment_len);
            if self.texts.get(slot.text_start as usize..end).is_none() {
                return Err(corrupt("slot points outside the text arena"));
            }
        }
        if self
            .keys
            .windows(2)
            .any(|w| self.code(w[0]) > self.code(w[1]))
        {
            return Err(corrupt("keys are not sorted"));
        }
        Ok(())
    }

    /// 从「编码 → 候选」建一个内存词库。编码重复时后者覆盖前者。
    ///
    /// 先整体排序再去重：输入的编码条数就是词库的规模（产品词库十万量级），
    /// 每来一条就在已有列表里线性找一遍是平方复杂度。
    pub fn from_items<I, C>(items: I) -> Self
    where
        I: IntoIterator<Item = (C, Vec<DictItem>)>,
        C: AsRef<str>,
    {
        let mut grouped: Vec<(Box<str>, Vec<DictItem>)> = items
            .into_iter()
            .map(|(code, candidates)| (code.as_ref().into(), candidates))
            .collect();
        // 稳定排序：编码相同时保持输入顺序，紧接着的去重就能让后者覆盖前者。
        grouped.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
        grouped.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                earlier.1 = std::mem::take(&mut later.1);
                true
            } else {
                false
            }
        });
        Self::from_sorted(grouped)
    }

    /// 已经按编码字节序排好、编码互不相同的输入。
    fn from_sorted(grouped: Vec<(Box<str>, Vec<DictItem>)>) -> Self {
        let mut texts = String::new();
        let mut codes = String::new();
        let mut keys = Vec::with_capacity(grouped.len());
        let mut slots = Vec::new();

        for (code, mut candidates) in grouped {
            // 同一个编码下按词频降序；稳定排序保证同词频的保持读入顺序。
            candidates.sort_by_key(|item| std::cmp::Reverse(item.weight));
            let first_slot = slots.len() as u32;
            for item in &candidates {
                slots.push(Slot {
                    text_start: texts.len() as u32,
                    weight: item.weight,
                    text_len: item.text.len() as u16,
                    comment_len: item.comment.as_ref().map_or(0, |c| c.len() as u16),
                });
                texts.push_str(&item.text);
                if let Some(comment) = &item.comment {
                    texts.push_str(comment);
                }
            }
            keys.push(KeyIndex {
                code_start: codes.len() as u32,
                first_slot,
                slot_count: candidates.len() as u32,
                code_len: code.len() as u16,
                reserved: 0,
            });
            codes.push_str(&code);
        }

        Self {
            texts: Text::Owned(texts),
            codes: Text::Owned(codes),
            keys: Table::Owned(keys),
            slots: Table::Owned(slots),
        }
    }

    /// 把内存词库写进一个容器文件（先写 `.tmp` 再改名）。
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), LiushuError> {
        format::write(
            path.as_ref(),
            KIND,
            &[
                Section {
                    tag: TEXT_TAG,
                    bytes: self.texts.as_bytes(),
                },
                Section {
                    tag: CODE_TAG,
                    bytes: self.codes.as_bytes(),
                },
                Section {
                    tag: KEY_TAG,
                    bytes: self.keys.as_bytes(),
                },
                Section {
                    tag: SLOT_TAG,
                    bytes: self.slots.as_bytes(),
                },
            ],
        )
    }

    /// 编码条数。
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// 候选条数。
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// 第 `index` 个编码（0 是字节序最小的那个）。
    pub fn code_at(&self, index: usize) -> &str {
        self.code(self.keys[index])
    }

    fn code(&self, key: KeyIndex) -> &str {
        let start = key.code_start as usize;
        &self.codes[start..start + usize::from(key.code_len)]
    }

    /// 第 `index` 个编码下的候选，已按词频降序。
    pub fn slots_at(&self, index: usize) -> &[Slot] {
        let key = self.keys[index];
        let start = key.first_slot as usize;
        &self.slots[start..start + key.slot_count as usize]
    }

    /// 候选词。
    pub fn slot_text(&self, slot: &Slot) -> &str {
        let start = slot.text_start as usize;
        &self.texts[start..start + usize::from(slot.text_len)]
    }

    /// 候选注释。
    pub fn slot_comment(&self, slot: &Slot) -> Option<&str> {
        if slot.comment_len == 0 {
            return None;
        }
        let start = slot.text_start as usize + usize::from(slot.text_len);
        Some(&self.texts[start..start + usize::from(slot.comment_len)])
    }

    /// 以 `prefix` 开头的编码在键表里的区间。
    ///
    /// 编码按字节序排好，所以「以某前缀开头」的键一定连续：区间起点是第一个 ≥ `prefix`
    /// 的键，终点是第一个 ≥ `prefix + 0xFF` 的键——UTF-8 里没有任何字节序列以 0xFF 开头，
    /// 所以带该前缀的键全都落在 `prefix + 0xFF` 之前。
    pub fn prefix_range(&self, prefix: &str) -> std::ops::Range<usize> {
        let start = self.partition_point(|code| code < prefix);
        let mut upper = Vec::with_capacity(prefix.len() + 1);
        upper.extend_from_slice(prefix.as_bytes());
        upper.push(0xFF);
        let end = match self.partition_point_bytes(&upper) {
            Some(end) => end,
            None => {
                // `prefix + 0xFF` 不是合法 UTF-8（前缀里出现非 ASCII 字节时）：退化成扫描，
                // 区间在键表里连续，逐条比到不匹配为止。
                let mut end = start;
                while end < self.keys.len() && self.code_at(end).starts_with(prefix) {
                    end += 1;
                }
                end
            }
        };
        start..end.max(start)
    }

    /// 第一个不小于 `needle` 的键的位置（按 UTF-8 字节序）。
    fn partition_point(&self, less: impl Fn(&str) -> bool) -> usize {
        let mut low = 0;
        let mut high = self.keys.len();
        while low < high {
            let mid = low + (high - low) / 2;
            if less(self.code_at(mid)) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    }

    /// 按原始字节比较的二分；`needle` 不是合法 UTF-8 时返回 `None`。
    fn partition_point_bytes(&self, needle: &[u8]) -> Option<usize> {
        std::str::from_utf8(needle).ok()?;
        let mut low = 0;
        let mut high = self.keys.len();
        while low < high {
            let mid = low + (high - low) / 2;
            if self.code_at(mid).as_bytes() < needle {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Some(low)
    }

    /// 有没有以 `prefix` 开头的编码。
    pub fn has_prefix(&self, prefix: &str) -> bool {
        !self.prefix_range(prefix).is_empty()
    }

    /// `code` 里最长的、能作为某个编码前缀的长度（按字节）；没有则 0。
    ///
    /// 单调性：`code[..len]` 是某个键的前缀，则所有更短的前缀也是，所以长度可以二分。
    pub fn longest_prefix_len(&self, code: &str) -> usize {
        let boundaries: Vec<usize> = code
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(code.len()))
            .filter(|&index| index > 0)
            .collect();
        let mut low = 0;
        let mut high = boundaries.len();
        while low < high {
            let mid = low + (high - low) / 2;
            if self.has_prefix(&code[..boundaries[mid]]) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if low == 0 { 0 } else { boundaries[low - 1] }
    }
}

/// 建词库：读若干 TSV（`text\tcode\tweight[\tcomment]`），写出容器文件。
///
/// 先按编码分组再做表：哈希探测比在键表上二分便宜，而且表每条编码只建一次。
pub fn build<I, O>(inputs: &[I], output: O) -> Result<(), LiushuError>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
{
    let mut grouped: HashMap<Box<str>, Vec<DictItem>> = HashMap::new();
    for dict_path in inputs {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .comment(Some(b'#'))
            .from_path(dict_path)?;
        for result in reader.deserialize() {
            let DictRow {
                text,
                code,
                weight,
                comment,
            } = result?;
            grouped.entry(code).or_default().push(DictItem {
                text,
                weight,
                comment,
            });
        }
    }

    let dictionary = Dictionary::from_items(grouped);
    dictionary.write(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, weight: u32) -> DictItem {
        DictItem {
            text: text.into(),
            weight,
            comment: None,
        }
    }

    fn sample() -> Dictionary {
        Dictionary::from_items([
            ("ni", vec![item("你", 10), item("尼", 5)]),
            (
                "nihao",
                vec![DictItem {
                    text: "你好".into(),
                    weight: 100,
                    comment: Some("greeting".into()),
                }],
            ),
            ("hao", vec![item("好", 50)]),
        ])
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("liushu-dict-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn codes_are_sorted_and_candidates_are_ranked() {
        let dictionary = sample();
        assert_eq!(dictionary.key_count(), 3);
        assert_eq!(dictionary.code_at(0), "hao");
        assert_eq!(dictionary.code_at(1), "ni");
        assert_eq!(dictionary.code_at(2), "nihao");
        let texts: Vec<&str> = dictionary
            .slots_at(1)
            .iter()
            .map(|slot| dictionary.slot_text(slot))
            .collect();
        assert_eq!(texts, ["你", "尼"]);
    }

    #[test]
    fn prefix_range_covers_every_code_with_that_prefix() {
        let dictionary = sample();
        assert_eq!(dictionary.prefix_range("ni"), 1..3);
        assert_eq!(dictionary.prefix_range("nihao"), 2..3);
        assert_eq!(dictionary.prefix_range("n"), 1..3);
        assert_eq!(dictionary.prefix_range("ha"), 0..1);
        assert!(dictionary.prefix_range("zzz").is_empty());
        assert!(dictionary.has_prefix("nih"));
        assert!(!dictionary.has_prefix("nihaoa"));
    }

    #[test]
    fn longest_prefix_len_finds_the_longest_known_prefix() {
        let dictionary = sample();
        assert_eq!(dictionary.longest_prefix_len("nihaoa"), 5);
        assert_eq!(dictionary.longest_prefix_len("nihke"), 3);
        assert_eq!(dictionary.longest_prefix_len("haoa"), 3);
        assert_eq!(dictionary.longest_prefix_len("ni"), 2);
        // 没有任何编码以 `k` 开头，整段都切不动。
        assert_eq!(dictionary.longest_prefix_len("keya"), 0);
        assert_eq!(dictionary.longest_prefix_len("zzz"), 0);
    }

    #[test]
    fn round_trips_through_a_file() {
        let path = temp_path("round-trip");
        sample().write(&path).unwrap();

        let mapped = Dictionary::open(&path).unwrap();
        assert_eq!(mapped.key_count(), 3);
        assert_eq!(mapped.slot_count(), 4);
        assert_eq!(mapped.code_at(2), "nihao");
        let slots = mapped.slots_at(2);
        assert_eq!(mapped.slot_text(&slots[0]), "你好");
        assert_eq!(mapped.slot_comment(&slots[0]), Some("greeting"));
        assert_eq!(mapped.slot_comment(&mapped.slots_at(1)[0]), None);
        assert_eq!(mapped.prefix_range("ni"), 1..3);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn an_empty_comment_is_stored_as_no_comment() {
        // arena 里注释紧跟在候选词后面，「空注释」与「没有注释」在长度上无法区分，
        // 统一按没有注释处理——候选旁的注释本来就是空的，没有可观察的区别。
        let path = temp_path("empty-comment");
        Dictionary::from_items([(
            "a",
            vec![DictItem {
                text: "啊".into(),
                weight: 1,
                comment: Some("".into()),
            }],
        )])
        .write(&path)
        .unwrap();
        let mapped = Dictionary::open(&path).unwrap();
        assert_eq!(mapped.slot_comment(&mapped.slots_at(0)[0]), None);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn duplicate_codes_keep_the_last_insert() {
        let dictionary =
            Dictionary::from_items([("ni", vec![item("你", 1)]), ("ni", vec![item("尼", 2)])]);
        assert_eq!(dictionary.key_count(), 1);
        assert_eq!(dictionary.slot_text(&dictionary.slots_at(0)[0]), "尼");
    }

    #[test]
    fn utf8_codes_are_ordered_by_bytes_not_chars() {
        // 中文编码虽然少见，但排序必须是字节序，否则前缀区间会错。
        let dictionary = Dictionary::from_items([
            ("你", vec![item("a", 1)]),
            ("你a", vec![item("b", 2)]),
            ("他", vec![item("c", 3)]),
        ]);
        let codes: Vec<&str> = (0..dictionary.key_count())
            .map(|index| dictionary.code_at(index))
            .collect();
        assert_eq!(codes, ["他", "你", "你a"]);
        assert_eq!(dictionary.prefix_range("你"), 1..3);
    }

    #[test]
    fn a_corrupt_index_is_rejected_on_open() {
        let path = temp_path("corrupt");
        let dictionary = sample();
        dictionary.write(&path).unwrap();

        let mut bytes = std::fs::read(&path).unwrap();
        let section_offset = |tag: &[u8; 4]| {
            let at = bytes.windows(4).position(|window| window == tag).unwrap();
            u64::from_le_bytes(bytes[at + 8..at + 16].try_into().unwrap()) as usize
        };
        let key_offset = section_offset(&KEY_TAG);
        // 把第一个 KeyIndex 的 slot_count 改到候选表外（slot_count 是结构体第三个字段）。
        bytes[key_offset + 8..key_offset + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(Dictionary::open(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
