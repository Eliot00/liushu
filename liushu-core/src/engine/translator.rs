use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::dict::Dictionary;

use super::candidates::Candidate;

pub trait Translator {
    fn translate(&self, code: &str) -> Vec<Candidate>;
}

/// 一次查询最多返回多少条候选。
///
/// 形码（五笔、郑码这些）一个键就能命中几千条编码：`w` 在 8.9 万条的码表里能拉出上万条，
/// 全部物化成 `Candidate`（每条至少两个 `String`）就是每次按键几 MB 的分配，而壳上一屏也
/// 就显示几十条。所以先只收借用视图，用 `select_nth` 取出权重最高的这一批，再物化。
pub const MAX_CANDIDATES: usize = 500;

/// 去重之后、还没物化成 `Candidate` 的一条候选。
struct View<'a> {
    text: &'a str,
    code: &'a str,
    comment: Option<&'a str>,
    weight: u32,

    /// 在键序里的名次，用来给同权重的候选定序（与旧实现「先出现的赢」一致）。
    order: usize,
}

impl Dictionary {
    /// 按前缀取候选，最多 `limit` 条。
    ///
    /// [`Translator::translate`] 用 [`MAX_CANDIDATES`] 调它；需要拿到全部命中（比如壳要做「翻到
    /// 底」或者自己统计）时传 `usize::MAX`。
    pub fn translate_limited(&self, code: &str, limit: usize) -> Vec<Candidate> {
        if code.is_empty() || limit == 0 {
            return vec![];
        }

        // 去重与选择都在借用视图上做：文本、编码、注释都不复制，等到最后确定要哪几条再物化。
        //
        // 同一个词可能在好几个编码下出现——形码的简码 / 全码 / 容错码，拼音的 `ni` / `nima`。
        // 候选只留一条：编码与注释取键序在前的那个（通常正是简码），**权重取最大的那个**——
        // 词的常用程度不该因为从哪个码打出来而变，否则简码行的低权重会把这个词压到后面。
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut views: Vec<View<'_>> = Vec::new();

        // 键表按字节序排好，以 `code` 开头的编码是一个连续区间，两次二分定位。
        for index in self.prefix_range(code) {
            let matched_code = self.code_at(index);
            for slot in self.slots_at(index) {
                let text = self.slot_text(slot);
                match seen.entry(text) {
                    Entry::Occupied(entry) => {
                        let view = &mut views[*entry.get()];
                        view.weight = view.weight.max(slot.weight());
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(views.len());
                        views.push(View {
                            text,
                            code: matched_code,
                            comment: self.slot_comment(slot),
                            weight: slot.weight(),
                            order: views.len(),
                        });
                    }
                }
            }
        }

        // 权重降序；同权重按键序，保证同一输入每次结果一样。
        let rank = |view: &View<'_>| (std::cmp::Reverse(view.weight), view.order);
        if views.len() > limit {
            // 只要前 N 条的话没必要整表排序：先划一刀，再对留下的一小撮排序。
            views.select_nth_unstable_by_key(limit, rank);
            views.truncate(limit);
        }
        views.sort_by_key(rank);

        views
            .into_iter()
            .map(|view| Candidate {
                text: view.text.to_owned(),
                code: view.code.to_owned(),
                comment: view.comment.map(str::to_owned),
                weight: view.weight,
            })
            .collect()
    }
}

impl Translator for Dictionary {
    fn translate(&self, code: &str) -> Vec<Candidate> {
        self.translate_limited(code, MAX_CANDIDATES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::DictItem;

    fn item(text: &str, weight: u32) -> DictItem {
        DictItem {
            text: text.into(),
            weight,
            comment: None,
        }
    }

    fn dict() -> Dictionary {
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

    #[test]
    fn empty_code_matches_nothing() {
        assert!(dict().translate("").is_empty());
    }

    #[test]
    fn unmatched_code_matches_nothing() {
        assert!(dict().translate("zzz").is_empty());
    }

    #[test]
    fn matches_every_code_carrying_the_prefix_ordered_by_weight() {
        let out = dict().translate("ni");
        let texts: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["你好", "你", "尼"]);
    }

    #[test]
    fn candidate_code_comes_from_the_matched_key() {
        let out = dict().translate("ni");
        assert_eq!(out[0].text, "你好");
        assert_eq!(out[0].code, "nihao");
        assert_eq!(out[1].text, "你");
        assert_eq!(out[1].code, "ni");
    }

    #[test]
    fn comment_survives() {
        let out = dict().translate("nihao");
        assert_eq!(out[0].comment.as_deref(), Some("greeting"));
    }

    #[test]
    fn dedups_by_text_keeping_the_first_code_and_the_best_weight() {
        // "你" lives under both "ni" and "nima". The candidate keeps the code that comes first in
        // key order ("ni", the shorter one) but the larger weight: how common a word is must not
        // depend on which of its codes the user typed.
        let d =
            Dictionary::from_items([("ni", vec![item("你", 10)]), ("nima", vec![item("你", 99)])]);

        let out = d.translate("ni");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].weight, 99);
        assert_eq!(out[0].code, "ni");
    }
}
