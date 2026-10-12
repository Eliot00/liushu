use crate::dict::Dictionary;

pub trait Segmentor {
    fn segment(&self, code: &str) -> Vec<String>;
}

impl Segmentor for Dictionary {
    fn segment(&self, code: &str) -> Vec<String> {
        let mut result = vec![];
        let mut last = 0;
        while last < code.len() {
            let lcp = self.longest_prefix_len(&code[last..]);
            if lcp > 0 {
                result.push(code[last..last + lcp].to_string());
                last += lcp;
            } else {
                result.push(code[last..].to_string());
                last = code.len();
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::DictItem;

    fn item(text: &str) -> DictItem {
        DictItem {
            text: text.into(),
            weight: 1,
            comment: None,
        }
    }

    #[test]
    fn test_split_pinyin() {
        let dictionary = Dictionary::from_items([
            ("nihao", vec![item("你好")]),
            ("ke", vec![item("可")]),
            ("yi", vec![item("以")]),
            ("a", vec![item("啊")]),
        ]);

        assert_eq!(dictionary.segment("nihaoa"), vec!["nihao", "a"]);
        assert_eq!(dictionary.segment("keyi"), vec!["ke", "yi"]);
        assert_eq!(dictionary.segment("aaaaa"), vec!["a", "a", "a", "a", "a"]);

        // empty
        assert_eq!(dictionary.segment(""), vec![] as Vec<String>);

        // unrecognized
        assert_eq!(dictionary.segment("keyide"), vec!["ke", "yi", "de"]);
        assert_eq!(dictionary.segment("ni"), vec!["ni"]);

        // partial
        assert_eq!(dictionary.segment("keya"), vec!["ke", "y", "a"]);
        assert_eq!(dictionary.segment("nihke"), vec!["nih", "ke"]);
    }
}
