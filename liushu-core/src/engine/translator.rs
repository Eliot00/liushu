use std::collections::HashSet;

use crate::dict::Dictionary;

use super::candidates::Candidate;

pub trait Translator {
    fn translate(&self, code: &str) -> Vec<Candidate>;
}

impl Translator for Dictionary {
    fn translate(&self, code: &str) -> Vec<Candidate> {
        if code.is_empty() {
            return vec![];
        }

        // Dedup on a borrowed view of the text. The previous `unique_by` had
        // to hand out an owned key, so it cloned every text on every keystroke
        // just to throw the copy away in the hash set.
        let mut seen: HashSet<&str> = HashSet::new();
        let mut candidates: Vec<Candidate> = Vec::new();

        // 键表按字节序排好，以 `code` 开头的编码是一个连续区间，一次二分就能定位。
        // 区间按键序推进，"先出现的赢"——与当初 `unique_by` 的行为一致。
        for index in self.prefix_range(code) {
            let matched_code = self.code_at(index);
            for slot in self.slots_at(index) {
                let text = self.slot_text(slot);
                if seen.insert(text) {
                    candidates.push(Candidate {
                        text: text.to_string(),
                        code: matched_code.to_string(),
                        comment: self.slot_comment(slot).map(str::to_owned),
                        weight: slot.weight(),
                    });
                }
            }
        }

        candidates.sort_by_key(|c| std::cmp::Reverse(c.weight));
        candidates
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
    fn dedups_by_text_keeping_the_first_in_prefix_order() {
        // "你" lives under both "ni" and "nima". Keys iterate in sorted order,
        // so "ni" is seen first and wins — matching the previous `unique_by`.
        let d =
            Dictionary::from_items([("ni", vec![item("你", 10)]), ("nima", vec![item("你", 99)])]);

        let out = d.translate("ni");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].weight, 10);
        assert_eq!(out[0].code, "ni");
    }
}
