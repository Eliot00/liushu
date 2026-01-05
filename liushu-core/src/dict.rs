use std::{fs::File, path::Path};

use patricia_tree::StringPatriciaMap;
use serde::{Deserialize, Serialize};

use crate::error::LiushuError;

pub type Dictionary = StringPatriciaMap<Vec<DictItem>>;

#[derive(Debug, Deserialize, Serialize)]
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

    let mut trie_writer = File::create(output)?;
    bincode_next::serde::encode_into_std_write(
        &trie,
        &mut trie_writer,
        bincode_next::config::standard().with_fixed_int_encoding(),
    )?;

    Ok(())
}
