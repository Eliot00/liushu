use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct State {
    pub active_formula_id: String,
    pub avaliable_formulas: Vec<Formula>,
}

impl State {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Self {
        let string = fs::read_to_string(path).unwrap();
        toml::from_str(&string).unwrap()
    }
}

impl From<&Config> for State {
    fn from(value: &Config) -> Self {
        Self {
            active_formula_id: value.formulas[0].id.clone(),
            avaliable_formulas: value
                .formulas
                .iter()
                .map(|f| Formula {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    db_path: f.get_db_path().display().to_string(),
                    use_hmm: f.use_hmm,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Formula {
    pub id: String,
    pub name: Option<String>,
    pub db_path: String,
    pub use_hmm: bool,
}
