use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_dhall::StaticType;

use crate::dirs::PROJECT_DIRS;

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

#[derive(Debug, Clone, Serialize, Deserialize, StaticType)]
pub struct Formula {
    pub id: String,
    pub name: Option<String>,
    pub use_hmm: bool,
    dictionaries: Vec<String>,
}

impl Formula {
    pub fn get_db_path(&self) -> PathBuf {
        let targe_dir = &PROJECT_DIRS.target_dir;
        targe_dir.join(format!("{}.db3", self.id))
    }

    pub fn get_dict_paths(&self) -> Vec<PathBuf> {
        let config_dir = &PROJECT_DIRS.config_dir;
        self.dictionaries
            .iter()
            .map(|dict| config_dir.join(&self.id).join(dict))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prelude() {
        let config = Config::load_from_path("../prelude/main.dhall");

        assert_eq!(config.formulas.len(), 2);

        let sunman = config.formulas[0].clone();
        assert_eq!(sunman.id, String::from("sunman"));
        assert_eq!(sunman.name, Some(String::from("山人全息")));

        assert_eq!(sunman.dictionaries.len(), 3);
        assert!(sunman.get_db_path().ends_with("sunman.db3"));
    }
}
