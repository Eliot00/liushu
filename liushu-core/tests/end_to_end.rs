//! 端到端：TSV 词库 + 语料 → 引擎 → 候选列表、切分、整句解码。
//!
//! 这条路径就是 `liushu-dict build` / `liushu-ngram train` 之后根 crate 启动时走的那条。
//! 单测覆盖的是 `Dictionary` / `LanguageModel` 各自的接口，这里覆盖 CLI 的契约：
//! TSV 的列顺序（含表头）、语料格式，以及装配之后能查到什么。

use std::path::PathBuf;

use liushu_core::{
    dict,
    engine::{Engine, InputMethodEngine, segmentor::Segmentor},
    lm,
};

fn temp_path(name: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "liushu-e2e-{name}-{}.{extension}",
        std::process::id()
    ))
}

#[test]
fn a_tsv_dictionary_builds_loads_and_answers_queries() {
    let tsv = temp_path("dict", "tsv");
    let dictionary = temp_path("dict", "bin");
    // 带表头，列顺序：词、编码、词频、注释（csv 要求每行字段数一致，没有注释就留空）。
    std::fs::write(
        &tsv,
        "text\tcode\tweight\tcomment\n你\tni\t10\t\n尼\tni\t5\t\n你好\tnihao\t100\tgreeting\n好\thao\t50\t\n",
    )
    .unwrap();

    dict::build(std::slice::from_ref(&tsv), &dictionary).unwrap();
    let engine = Engine::new(&dictionary).unwrap();

    let texts: Vec<String> = engine
        .search("ni")
        .unwrap()
        .into_iter()
        .map(|candidate| candidate.text)
        .collect();
    assert_eq!(texts, ["你好", "你", "尼"]);

    let first = engine.search("nihao").unwrap();
    assert_eq!(first[0].code, "nihao");
    assert_eq!(first[0].comment.as_deref(), Some("greeting"));

    assert_eq!(engine.segment("nihaoni"), ["nihao", "ni"]);

    // 没有语言模型时解不出整句，但候选列表照常。
    assert!(engine.sentences("nihao", 3).is_empty());

    std::fs::remove_file(&tsv).unwrap();
    std::fs::remove_file(&dictionary).unwrap();
}

#[test]
fn a_trained_model_lets_the_engine_decode_a_sentence() {
    let tsv = temp_path("sentence", "tsv");
    let dictionary = temp_path("sentence", "bin");
    let corpus = temp_path("sentence", "corpus");
    let model_path = temp_path("model", "bin");

    // 两种切分能拼出不同的句子：`nihao` 一个词是「你好」，`ni` + `hao` 是「你号」。
    std::fs::write(
        &tsv,
        "text\tcode\tweight\tcomment\n你好\tnihao\t100\t\n你\tni\t10\t\n号\thao\t10\t\n",
    )
    .unwrap();
    std::fs::write(
        &corpus,
        format!(
            "{}{}",
            "你好\tnihao\n".repeat(100),
            "你 号\tni hao\n".repeat(10)
        ),
    )
    .unwrap();

    dict::build(std::slice::from_ref(&tsv), &dictionary).unwrap();
    lm::train(&corpus, &model_path).unwrap();

    let engine = Engine::with_model(&dictionary, &model_path).unwrap();
    let sentences = engine.sentences("nihao", 2);
    assert_eq!(
        sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>(),
        ["你好", "你号"]
    );
    assert!(sentences[0].score > sentences[1].score);

    // 音节分开写与连着写等价。
    assert_eq!(engine.sentences("ni hao", 1)[0].text, "你好");
    // 切不动的输入没有整句，也不会 panic。
    assert!(engine.sentences("nihaoq", 2).is_empty());

    for path in [tsv, dictionary, corpus, model_path] {
        std::fs::remove_file(path).unwrap();
    }
}
