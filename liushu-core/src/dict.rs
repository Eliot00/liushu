use std::{collections::HashSet, fs::File, path::Path};

use boomphf::Mphf;
use itertools::Itertools;
use patricia_tree::{StringPatriciaMap, StringPatriciaSet};
use serde::{Deserialize, Serialize};

use crate::error::LiushuError;

pub type Dictionary = StringPatriciaMap<Vec<DictItem>>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DictItem {
    pub text: String,
    pub code: String,
    pub weight: u32,
    pub comment: Option<String>,
}

pub fn build<I, O>(inputs: &Vec<I>, output: O) -> Result<(), LiushuError>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
{
    let mut trie = StringPatriciaMap::new();
    for dict_path in inputs {
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .comment(Some(b'#'))
            .from_path(dict_path)?;
        for result in rdr.deserialize() {
            let item: DictItem = result?;
            let code = item.code.clone();

            if trie.get(&code).is_none() {
                trie.insert(&code, vec![item]);
            } else if let Some(entry) = trie.get_mut(code.as_str()) {
                entry.push(item);
            }
        }
    }

    let trie_writer = File::create(output)?;
    bincode::serialize_into(trie_writer, &trie)?;

    Ok(())
}

pub fn build2<I, O>(inputs: &[I], output_dir: O) -> Result<(), LiushuError>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
{
    let mut items = Vec::new();
    for dict_path in inputs {
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .comment(Some(b'#'))
            .from_path(dict_path)?;
        for result in rdr.deserialize() {
            let item: DictItem = result?;
            items.push(item.clone());
        }
    }

    let mut trie: StringPatriciaMap<StringPatriciaSet> = StringPatriciaMap::new();
    let uniq_words_vec: Vec<String> = items.iter().map(|i| i.text.clone()).unique().collect();
    let phf = Mphf::new(1.7, &uniq_words_vec);
    let mut def_table = vec![(0, None); uniq_words_vec.len()];
    let mut visited = HashSet::new();
    for item in items {
        let code = &item.code;
        if let Some(entry) = trie.get_mut(code) {
            entry.insert(item.text.clone());
        } else {
            trie.insert(item.code, StringPatriciaSet::from_iter([item.text.clone()]));
        }
        if visited.get(&item.text).is_none() {
            visited.insert(item.text.clone());
            let index = phf.hash(&item.text);
            def_table[index as usize] = (item.weight.clone(), item.comment.clone());
        }
    }

    let output_dir = output_dir.as_ref();
    let trie_writer = File::create(output_dir.join("index.bin"))?;
    bincode::serialize_into(trie_writer, &trie)?;

    let def_writer = File::create(output_dir.join("def.bin"))?;
    bincode::serialize_into(def_writer, &def_table)?;

    Ok(())
}
