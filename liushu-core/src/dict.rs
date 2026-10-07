use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use patricia_tree::StringPatriciaMap;
use serde::{Deserialize, Serialize};

use crate::error::LiushuError;

/// Keyed by input code.
///
/// The code is stored once, as the map key, rather than being repeated inside
/// every item that shares it.
pub type Dictionary = StringPatriciaMap<Vec<DictItem>>;

/// A single candidate.
///
/// The input code is deliberately absent: it is the key this item is stored
/// under in the [`Dictionary`]. Keeping a copy here duplicated every key once
/// per candidate (the largest single consumer of memory in the trie).
///
/// `Box<str>` instead of `String` drops the spare capacity field, which is
/// dead weight for strings that are never mutated after construction.
#[derive(Debug, Deserialize, Serialize)]
pub struct DictItem {
    pub text: Box<str>,
    pub weight: u32,
    pub comment: Option<Box<str>>,
}

/// One row of the input TSV.
///
/// Unlike [`DictItem`] this carries the code, because at this point the row
/// has not been keyed into the trie yet and the code is what keys it.
#[derive(Debug, Deserialize)]
struct DictRow {
    text: Box<str>,
    code: Box<str>,
    weight: u32,
    comment: Option<Box<str>>,
}

pub fn build<I, O>(inputs: &Vec<I>, output: O) -> Result<(), LiushuError>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
{
    let mut trie: Dictionary = StringPatriciaMap::new();
    for dict_path in inputs {
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .comment(Some(b'#'))
            .from_path(dict_path)?;
        for result in rdr.deserialize() {
            let DictRow {
                text,
                code,
                weight,
                comment,
            } = result?;
            let item = DictItem {
                text,
                weight,
                comment,
            };

            // One trie walk: `get_mut` both tests for the key and hands back
            // the entry to append to.
            match trie.get_mut(code.as_ref()) {
                Some(entry) => entry.push(item),
                None => {
                    trie.insert(code.as_ref(), vec![item]);
                }
            }
        }
    }

    // bincode emits many small writes; without buffering each one is its own
    // syscall, which dominated the whole build (508ms of a 537ms run on an
    // 88k-row corpus, versus 2ms buffered).
    let mut trie_writer = BufWriter::new(File::create(output)?);
    bincode_next::serde::encode_into_std_write(
        &trie,
        &mut trie_writer,
        bincode_next::config::standard().with_fixed_int_encoding(),
    )?;
    // Flush explicitly: a `BufWriter` dropped on error would swallow the error.
    trie_writer.flush()?;

    Ok(())
}
