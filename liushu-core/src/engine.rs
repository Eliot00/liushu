pub mod candidates;
pub mod segmentor;
pub mod translator;

use std::{fs::File, path::Path};

use crate::{dict::Dictionary, error::LiushuError};

use self::{candidates::Candidate, segmentor::Segmentor, translator::Translator};

pub trait InputMethodEngine {
    fn search(&self, code: &str) -> Result<Vec<Candidate>, LiushuError>;
}

#[derive(Debug, Default)]
pub struct Engine {
    trie: Dictionary,
}

impl Engine {
    pub fn new(dict_path: impl AsRef<Path>) -> Result<Self, LiushuError> {
        let mut dict_file = File::open(dict_path)?;
        let trie: Dictionary = bincode_next::serde::decode_from_std_read(
            &mut dict_file,
            bincode_next::config::standard().with_fixed_int_encoding(),
        )?;

        Ok(Self { trie })
    }
}

impl InputMethodEngine for Engine {
    fn search(&self, code: &str) -> Result<Vec<Candidate>, LiushuError> {
        Ok(self.trie.translate(code))
    }
}

impl Segmentor for Engine {
    fn segment(&self, code: &str) -> Vec<String> {
        self.trie.segment(code)
    }
}
