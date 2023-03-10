use std::{fs::File, path::Path};

use patricia_tree::PatriciaMap;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_dhall::StaticType;

use crate::{
    dict::{DictItem, DICTIONARY},
    dirs::PROJECT_DIRS,
    error::LiushuError,
};

#[derive(Debug, Serialize, Deserialize, StaticType)]
pub struct Config {
    pub formulas: Vec<Formula>,
}

impl Config {
    pub fn load() -> Self {
        Self::load_from_path(PROJECT_DIRS.config_dir.join("main.dhall"))
    }

    fn load_from_path<P: AsRef<Path>>(path: P) -> Self {
        serde_dhall::from_file(path)
            .static_type_annotation()
            .parse()
            .unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize, StaticType)]
pub struct Formula {
    pub id: String,
    name: Option<String>,
    dictionaries: Vec<String>,
}

impl Formula {
    pub fn compile(
        &self,
        config_base_dir: impl AsRef<Path>,
        target_dir: impl AsRef<Path>,
    ) -> Result<(), LiushuError> {
        let self_config_dir = config_base_dir.as_ref().join(&self.id);
        let db_path = target_dir.as_ref().join(format!("{}.db3", self.id));
        let mut conn = Connection::open(db_path)?;
        let tx = conn.transaction()?;
        for dict_path in &self.dictionaries {
            let dict_path = self_config_dir.join(dict_path);
            let mut rdr = csv::ReaderBuilder::new()
                .delimiter(b'\t')
                .comment(Some(b'#'))
                .from_path(dict_path)?;
            for result in rdr.deserialize() {
                let dict: DictItem = result?;
                tx.execute(
                    "INSERT INTO dict (text, code, weight, comment) VALUES (?1, ?2, ?3, ?4)",
                    params![dict.text, dict.code, dict.weight, dict.comment],
                )?;
            }
        }
        Ok(())
    }

    pub fn compile2(
        &self,
        config_base_dir: impl AsRef<Path>,
        target_dir: impl AsRef<Path>,
    ) -> Result<(), LiushuError> {
        let self_config_dir = config_base_dir.as_ref().join(&self.id);
        let db_path = target_dir.as_ref().join(format!("{}.redb", self.id));

        let table = redb::Database::create(db_path)?;
        let tx = table.begin_write()?;
        let mut trie = PatriciaMap::new();
        {
            let mut dict_table = tx.open_table(DICTIONARY)?;
            for dict_path in &self.dictionaries {
                let dict_path = self_config_dir.join(dict_path);
                let mut rdr = csv::ReaderBuilder::new()
                    .delimiter(b'\t')
                    .comment(Some(b'#'))
                    .from_path(dict_path)?;
                for result in rdr.deserialize() {
                    let DictItem {
                        text,
                        code,
                        weight,
                        comment,
                    } = result?;
                    dict_table.insert(text.as_str(), (weight, comment.as_deref()))?;

                    if trie.get(&code).is_none() {
                        trie.insert_str(code.as_str(), vec![text]);
                    } else if let Some(entry) = trie.get_mut(code.as_str()) {
                        entry.push(text);
                    }
                }
            }
        }
        tx.commit()?;

        let trie_path = target_dir.as_ref().join(format!("{}.trie", self.id));
        let trie_writer = File::create(trie_path)?;
        bincode::serialize_into(trie_writer, &trie)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Clone for Formula {
        fn clone(&self) -> Self {
            Self {
                id: self.id.clone(),
                name: self.name.clone(),
                dictionaries: self.dictionaries.clone(),
            }
        }
    }

    #[test]
    fn test_prelude() {
        let config = Config::load_from_path("../prelude/main.dhall");

        assert_eq!(config.formulas.len(), 1);

        let sunman = config.formulas[0].clone();
        assert_eq!(sunman.id, String::from("sunman"));
        assert_eq!(sunman.name, Some(String::from("????????????")));

        assert_eq!(sunman.dictionaries.len(), 3);
    }
}
