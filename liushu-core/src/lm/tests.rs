//! 语言模型的往返与概率测试。

use super::*;

/// 语料：`我想去` 出现两次、`我想吃` 一次，`想吃` 这个词对只有一次。
const CORPUS: &str = "我 想 去\two xiang qu\n我 想 去\two xiang qu\n我 想 吃\two xiang chi\n";

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("liushu-lm-{name}-{}.bin", std::process::id()))
}

fn corpus_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("liushu-lm-{name}-{}.tsv", std::process::id()));
    std::fs::write(&path, CORPUS).unwrap();
    path
}

/// 训练 → 落盘 → 打开，返回模型与两个临时文件路径。
fn trained(name: &str) -> (LanguageModel, std::path::PathBuf, std::path::PathBuf) {
    let corpus = corpus_path(name);
    let model_path = temp_path(name);
    train(&corpus, &model_path).unwrap();
    let model = LanguageModel::open(&model_path).unwrap();
    (model, corpus, model_path)
}

#[test]
fn the_hash_index_finds_every_word_and_rejects_unknown_ones() {
    let (model, corpus, model_path) = trained("hash");
    for word in ["我", "想", "去", "吃"] {
        let id = model.word_id(word).unwrap();
        assert_eq!(model.word(id), word);
    }
    assert_eq!(model.word_id("火星"), None);
    assert_eq!(model.word_id(""), None);
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn interpolation_shrinks_the_bigram_toward_the_unigram() {
    let (model, corpus, model_path) = trained("interpolation");
    // 我 → 想 每次都出现：二阶项是 1，插值后 0.8 + 0.2·P(想)。
    let unigram = model.unigram_log_prob("想").unwrap();
    let expected = (LAMBDA * 1.0 + (1.0 - LAMBDA) * dequantize(unigram).exp()).ln();
    let actual = dequantize(model.log_prob(Some("我"), "想").unwrap());
    // Q4 的量化误差是 ±1/32 log 单位，留 0.05 的余量。
    assert!((actual - expected).abs() < 0.05, "{actual} vs {expected}");

    // 前一个词不在模型里：退化成纯一元概率。
    assert_eq!(
        model.log_prob(Some("火星"), "想"),
        model.unigram_log_prob("想")
    );
    // 句首同样只看一元概率。
    assert_eq!(model.log_prob(None, "想"), model.unigram_log_prob("想"));
    // 模型里没有的词返回 None。
    assert_eq!(model.log_prob(Some("我"), "火星"), None);
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn a_pruned_pair_falls_back_to_the_same_value_as_an_unseen_one() {
    let (model, corpus, model_path) = trained("pruned");
    // 想 → 吃 只出现一次、被剪掉；取值必须与「c(v,w)=0」的插值完全一致：ln P(吃) + ln(0.2)。
    let unigram = model.unigram_log_prob("吃").unwrap();
    assert_eq!(
        model.log_prob(Some("想"), "吃"),
        Some(unigram + LN_ONE_MINUS_LAMBDA)
    );
    // 反过来，真实存在的词对不能是这个兜底值。
    assert_ne!(
        model.log_prob(Some("我"), "想"),
        Some(model.unigram_log_prob("想").unwrap() + LN_ONE_MINUS_LAMBDA)
    );
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn a_more_likely_continuation_scores_higher() {
    let (model, corpus, model_path) = trained("ordering");
    let qu = model.log_prob(Some("想"), "去").unwrap();
    let chi = model.log_prob(Some("想"), "吃").unwrap();
    assert!(qu > chi, "{qu} should beat {chi}");
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn the_mapped_model_answers_exactly_like_the_trained_one() {
    let corpus = corpus_path("round-trip");
    let first = temp_path("round-trip-a");
    let second = temp_path("round-trip-b");

    let counts = count::count(&corpus).unwrap();
    let trained_model = from_counts(&counts);
    trained_model.write(&first).unwrap();
    let mapped = LanguageModel::open(&first).unwrap();

    assert_eq!(mapped.word_count(), trained_model.word_count());
    assert_eq!(mapped.transition_count(), trained_model.transition_count());
    for word in ["我", "想", "去", "吃"] {
        assert_eq!(mapped.word_id(word), trained_model.word_id(word));
        assert_eq!(
            mapped.unigram_log_prob(word),
            trained_model.unigram_log_prob(word)
        );
    }
    for previous in ["我", "想", "去", "吃"] {
        for word in ["我", "想", "去", "吃"] {
            assert_eq!(
                mapped.log_prob(Some(previous), word),
                trained_model.log_prob(Some(previous), word)
            );
        }
    }

    // 重新训练一次，字节相同：编号顺序只取决于语料，不取决于哈希顺序。
    train(&corpus, &second).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );

    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&first).unwrap();
    std::fs::remove_file(&second).unwrap();
}

#[test]
fn quantizing_keeps_the_ordering_of_close_probabilities() {
    // Q4 的分辨率是 1/16 log 单位：四舍五入后仍然单调，且 0 与 -1 不会撞在一起。
    assert_eq!(quantize(0.0), 0);
    assert_eq!(quantize(-1.0), -16);
    assert_eq!(quantize(-0.03), 0);
    assert_eq!(quantize(-0.04), -1);
    assert!(quantize(-1.0) > quantize(-2.0));
    assert!((dequantize(-16) + 1.0).abs() < 1e-9);
}

#[test]
fn the_stable_hash_is_pinned() {
    // 换哈希函数会让已有文件的索引全部失效，测试钉死输出值。
    assert_eq!(stable_hash(""), 0xefd0_1f60_ba99_2926);
    assert_eq!(stable_hash("开发"), 0x3758_0f72_eb7d_16d3);
}

#[test]
fn an_empty_corpus_is_an_error() {
    let corpus = std::env::temp_dir().join(format!("liushu-lm-empty-{}.tsv", std::process::id()));
    std::fs::write(&corpus, "# 只有注释\n").unwrap();
    assert!(train(&corpus, temp_path("empty")).is_err());
    std::fs::remove_file(&corpus).unwrap();
}

#[test]
fn a_truncated_file_is_rejected() {
    let (_, corpus, model_path) = trained("truncated");
    let bytes = std::fs::read(&model_path).unwrap();
    std::fs::write(&model_path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(LanguageModel::open(&model_path).is_err());
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn a_dictionary_file_is_not_a_language_model() {
    // 拿词库容器当模型打开：kind 不符要报错，而不是拿错字节去解释。
    let path = temp_path("wrong-kind");
    format::write(
        &path,
        *b"DICT",
        &[Section {
            tag: *b"TEXT",
            bytes: b"",
        }],
    )
    .unwrap();
    assert!(LanguageModel::open(&path).is_err());
    std::fs::remove_file(&path).unwrap();
}
