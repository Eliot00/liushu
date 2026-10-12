pub mod candidates;
pub mod segmentor;
pub mod translator;

use std::path::Path;

use crate::{dict::Dictionary, error::LiushuError};

use self::{candidates::Candidate, segmentor::Segmentor, translator::Translator};

pub trait InputMethodEngine {
    fn search(&self, code: &str) -> Result<Vec<Candidate>, LiushuError>;
}

#[derive(Debug, Default)]
pub struct Engine {
    dictionary: Dictionary,
}

impl Engine {
    pub fn new(dict_path: impl AsRef<Path>) -> Result<Self, LiushuError> {
        // 打开容器只是 mmap 再校验一遍头与分节边界，没有反序列化，加载耗时与文件大小无关。
        Ok(Self {
            dictionary: Dictionary::open(dict_path)?,
        })
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
