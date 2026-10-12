//! 语言模型的往返与解码测试。

use super::*;

/// 语料：`我想去` 出现两次、`我想吃` 一次，`想吃` 这个二元组只有一次。
const CORPUS: &str = "我 想 去\two xiang qu\n我 想 去\two xiang qu\n我 想 吃\two xiang chi\n";

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("liushu-lm-{name}-{}.bin", std::process::id()))
}

fn corpus_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("liushu-lm-{name}-{}.tsv", std::process::id()));
    std::fs::write(&path, CORPUS).unwrap();
    path
}

fn pinyins(sentence: &[&str]) -> Vec<String> {
    sentence.iter().map(|pinyin| pinyin.to_string()).collect()
}

/// 训练 → 落盘 → 打开，返回可查询的模型。
fn trained(name: &str) -> (LanguageModel, std::path::PathBuf, std::path::PathBuf) {
    let corpus = corpus_path(name);
    let model_path = temp_path(name);
    train(&corpus, &model_path).unwrap();
    let model = LanguageModel::open(&model_path).unwrap();
    (model, corpus, model_path)
}

#[test]
fn decodes_the_more_likely_sentence() {
    let (model, corpus, model_path) = trained("decode");
    assert_eq!(
        model.sentence(&pinyins(&["wo", "xiang", "qu"])).unwrap(),
        "我想去"
    );
    // `想吃` 这个二元组被剪掉了（只出现一次），但该位置只有一个候选词，兜底分数不影响结果。
    assert_eq!(
        model.sentence(&pinyins(&["wo", "xiang", "chi"])).unwrap(),
        "我想吃"
    );
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn a_syllable_with_no_candidate_is_an_error() {
    let (model, corpus, model_path) = trained("unknown");
    let error = model
        .sentence(&pinyins(&["wo", "xiang", "zzz"]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("zzz"), "{error}");
    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&model_path).unwrap();
}

#[test]
fn an_empty_pinyin_sequence_decodes_to_nothing() {
    let (model, corpus, model_path) = trained("empty");
    assert_eq!(model.sentence(&[]).unwrap(), "");
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
    for pinyin in ["wo", "xiang", "qu", "chi"] {
        let trained = trained_model.emissions_for(pinyin).unwrap().to_vec();
        let from_file = mapped.emissions_for(pinyin).unwrap();
        assert_eq!(trained.len(), from_file.len());
        for (a, b) in trained.iter().zip(from_file) {
            assert_eq!(a.word, b.word);
            assert_eq!(a.log_prob, b.log_prob);
        }
    }
    for (previous, word) in [(0u32, 1u32), (1, 2), (2, 3)] {
        assert_eq!(
            mapped.transition_log_prob(previous, word),
            trained_model.transition_log_prob(previous, word)
        );
    }
    assert_eq!(
        mapped.sentence(&pinyins(&["wo", "xiang", "qu"])).unwrap(),
        "我想去"
    );

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
fn rare_transitions_are_pruned_and_common_ones_survive() {
    let corpus = corpus_path("pruned");
    let path = temp_path("pruned");
    train(&corpus, &path).unwrap();
    let model = LanguageModel::open(&path).unwrap();

    let word = |text: &str| -> u32 {
        (0..model.word_count() as u32)
            .find(|&id| model.word(id) == text)
            .unwrap()
    };
    // 我 → 想 出现两次，保留。
    assert!(model.transition_log_prob(word("我"), word("想")).is_some());
    // 想 → 吃 只出现一次，剪掉。
    assert_eq!(model.transition_log_prob(word("想"), word("吃")), None);
    // 剪枝后的表里只剩一条转移。
    assert_eq!(model.transition_count(), 2);

    std::fs::remove_file(&corpus).unwrap();
    std::fs::remove_file(&path).unwrap();
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
