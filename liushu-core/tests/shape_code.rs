//! 形码（五笔 / 郑码 / 笔画这类）场景：编码是任意字节串，按前缀逐键取候选。
//!
//! 词库对编码不做任何假设——不进音节表、不切分、不转小写，只按 UTF-8 字节序排序，所以
//! 拼音码表、五笔码表、数字笔画码表走的是同一套查询路径。这个文件把形码真正用到的那几件事
//! 钉死：逐键前缀、同权重定序、同词多码去重、精确匹配、以及一键命中上万条时的限流。

use liushu_core::{
    dict::{DictItem, Dictionary},
    engine::{
        Engine, InputMethodEngine,
        candidates::Candidate,
        translator::{MAX_CANDIDATES, Translator},
    },
};

fn item(text: &str, weight: u32) -> DictItem {
    DictItem {
        text: text.into(),
        weight,
        comment: None,
    }
}

/// 一份五笔 86 风格的小码表：一简 / 二简 / 三简 / 四码混在一起，权重各不相同。
fn wubi() -> Dictionary {
    Dictionary::from_items([
        ("w", vec![item("人", 5)]),
        ("wqiy", vec![item("你", 100)]),
        ("wun", vec![item("们", 90)]),
        ("trnt", vec![item("我", 80)]),
        ("vbg", vec![item("好", 70)]),
        ("khk", vec![item("中", 60)]),
        ("lgyi", vec![item("国", 50)]),
        ("wwww", vec![item("人", 40)]),
        ("wty", vec![item("八", 30)]),
        ("www", vec![item("众", 20)]),
        ("wfh", vec![item("什", 10)]),
    ])
}

fn texts(candidates: &[Candidate]) -> Vec<&str> {
    candidates.iter().map(|c| c.text.as_str()).collect()
}

#[test]
fn every_keystroke_narrows_the_prefix_range() {
    let dictionary = wubi();
    // 一键：所有以 `w` 开头的编码，按词频降序。`人` 同时有简码 `w`(5) 与全码 `wwww`(40)，
    // 去重时权重取大的那个，所以它排在第三。
    assert_eq!(
        texts(&dictionary.translate("w")),
        ["你", "们", "人", "八", "众", "什"]
    );
    // 二键、三键、四键逐级收窄。
    assert_eq!(texts(&dictionary.translate("wq")), ["你"]);
    assert_eq!(texts(&dictionary.translate("wqi")), ["你"]);
    assert_eq!(texts(&dictionary.translate("wqiy")), ["你"]);
    // 键比任何编码都长：没有候选，但不是错误。
    assert!(dictionary.translate("wqiyx").is_empty());
    // 别的键起头。
    assert_eq!(texts(&dictionary.translate("t")), ["我"]);
    assert_eq!(texts(&dictionary.translate("v")), ["好"]);
}

#[test]
fn candidates_carry_the_code_that_matched() {
    let dictionary = wubi();
    let candidates = dictionary.translate("ww");
    // `ww` 命中 `www`（众）与 `wwww`（人），编码字段报的是命中的那个键。
    assert_eq!(
        candidates
            .iter()
            .map(|c| (c.text.as_str(), c.code.as_str(), c.weight))
            .collect::<Vec<_>>(),
        [("人", "wwww", 40), ("众", "www", 20)]
    );
}

#[test]
fn the_same_word_under_a_short_and_a_full_code_is_one_candidate() {
    // 五笔码表里同一个字既有简码又有全码：候选只留一条，编码取字节序在前的那个（简码），
    // 权重取两条里更大的那个。
    let dictionary =
        Dictionary::from_items([("w", vec![item("人", 5)]), ("wwww", vec![item("人", 40)])]);
    let candidates = dictionary.translate("w");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].code, "w");
    assert_eq!(candidates[0].weight, 40);
}

#[test]
fn exact_lookup_is_available_for_auto_commit_style_checks() {
    let dictionary = wubi();
    // 整码命中：形码壳可以据此判断「唯一候选 / 自动上屏」。
    assert!(dictionary.exact("wqiy").is_some());
    assert!(dictionary.exact("wqi").is_none());
    let index = dictionary.exact("wqiy").unwrap();
    assert_eq!(dictionary.slots_at(index).len(), 1);
    assert_eq!(dictionary.slot_text(&dictionary.slots_at(index)[0]), "你");
}

#[test]
fn equal_weights_keep_the_key_order_deterministically() {
    let dictionary = Dictionary::from_items([
        ("aa", vec![item("甲", 7)]),
        ("ab", vec![item("乙", 7)]),
        ("ac", vec![item("丙", 7)]),
    ]);
    let decoded = dictionary.translate("a");
    let first = texts(&decoded);
    assert_eq!(first, ["甲", "乙", "丙"]);
    for _ in 0..5 {
        assert_eq!(texts(&dictionary.translate("a")), first);
    }
}

#[test]
fn codes_need_not_be_letters() {
    // 笔画码 / 带数字的码表：编码只是字节串，前缀区间照样成立。
    let dictionary = Dictionary::from_items([
        ("1", vec![item("一", 10)]),
        ("12", vec![item("二", 9)]),
        ("123", vec![item("三", 8)]),
        ("12345", vec![item("正", 7)]),
        ("21", vec![item("丨", 6)]),
    ]);
    assert_eq!(texts(&dictionary.translate("1")), ["一", "二", "三", "正"]);
    assert_eq!(texts(&dictionary.translate("12")), ["二", "三", "正"]);
    assert_eq!(texts(&dictionary.translate("2")), ["丨"]);
}

#[test]
fn a_huge_prefix_range_is_capped_but_keeps_the_best_candidates() {
    // 8 万个四码编码全部以 `a` 开头，权重随序号递增：一键就能命中整张表。
    let dictionary = Dictionary::from_items(
        (0..80_000u32)
            .map(|index| {
                (
                    format!("a{index:05}"),
                    vec![item(&format!("词{index}"), index + 1)],
                )
            })
            .collect::<Vec<_>>(),
    );
    let candidates = dictionary.translate("a");
    assert_eq!(candidates.len(), MAX_CANDIDATES);
    // 留下的必须是权重最高的那一批。
    assert_eq!(candidates[0].weight, 80_000);
    assert!(
        candidates
            .windows(2)
            .all(|pair| pair[0].weight >= pair[1].weight)
    );
}

#[test]
fn the_cap_can_be_lifted_when_the_shell_wants_everything() {
    let dictionary = Dictionary::from_items(
        (0..1_200u32)
            .map(|index| {
                (
                    format!("a{index:04}"),
                    vec![item(&format!("词{index}"), index + 1)],
                )
            })
            .collect::<Vec<_>>(),
    );
    // 默认封顶。
    assert_eq!(dictionary.translate("a").len(), MAX_CANDIDATES);
    // 想要全部就给一个足够大的上限。
    let all = dictionary.translate_limited("a", usize::MAX);
    assert_eq!(all.len(), 1_200);
    assert_eq!(all[0].weight, 1_200);
    // 上限为 0 就是不要候选。
    assert!(dictionary.translate_limited("a", 0).is_empty());
}

#[test]
fn a_wubi_table_goes_through_the_engine_like_any_other() {
    // 装配成 Engine（也就是根 crate 用的那条路径）之后行为一致。
    let path = std::env::temp_dir().join(format!("liushu-shape-{}.bin", std::process::id()));
    wubi().write(&path).unwrap();
    let engine = Engine::new(&path).unwrap();
    assert_eq!(texts(&engine.search("wqi").unwrap()), ["你"]);
    assert_eq!(texts(&engine.search("khk").unwrap()), ["中"]);
    std::fs::remove_file(&path).unwrap();
}
