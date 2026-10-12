pub mod candidates;
pub mod segmentor;
pub mod sentence;
pub mod translator;

use std::path::Path;

use crate::{dict::Dictionary, error::LiushuError, lm::LanguageModel};

use self::{
    candidates::Candidate, segmentor::Segmentor, sentence::Sentence, translator::Translator,
};

pub trait InputMethodEngine {
    fn search(&self, code: &str) -> Result<Vec<Candidate>, LiushuError>;
}

#[derive(Debug, Default)]
pub struct Engine {
    dictionary: Dictionary,

    /// 词级语言模型是可选的：没有它，候选列表照常工作，只是解不出整句。
    model: Option<LanguageModel>,
}

impl Engine {
    /// 只装词库：候选列表能用，整句解码不可用。
    pub fn new(dict_path: impl AsRef<Path>) -> Result<Self, LiushuError> {
        Ok(Self {
            dictionary: Dictionary::open(dict_path)?,
            model: None,
        })
    }

    /// 词库 + 词级语言模型：整句解码用它给「词接词」打分。
    pub fn with_model(
        dict_path: impl AsRef<Path>,
        model_path: impl AsRef<Path>,
    ) -> Result<Self, LiushuError> {
        Ok(Self {
            dictionary: Dictionary::open(dict_path)?,
            model: Some(LanguageModel::open(model_path)?),
        })
    }

    /// 整句候选，按分数从高到低，最多 `k` 条。
    ///
    /// 没有装语言模型、输入切不出音节、词图走不通时返回空列表；这两种情况调用方都应该原样
    /// 落回 [`InputMethodEngine::search`] 的候选。
    pub fn sentences(&self, code: &str, k: usize) -> Vec<Sentence> {
        match &self.model {
            Some(model) => sentence::decode(&self.dictionary, model, code, k),
            None => Vec::new(),
        }
    }

    /// 装载的词库，供需要自己查词的调用方使用。
    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }
}

impl InputMethodEngine for Engine {
    fn search(&self, code: &str) -> Result<Vec<Candidate>, LiushuError> {
        Ok(self.dictionary.translate(code))
    }
}

impl Segmentor for Engine {
    fn segment(&self, code: &str) -> Vec<String> {
        self.dictionary.segment(code)
    }
}
