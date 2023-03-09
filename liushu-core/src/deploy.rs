use std::fs;

use crate::{config::Config, dict::compile_dicts_to_db, dirs::PROJECT_DIRS, engine::state::State};

pub fn deploy() {
    let config = Config::load();

    config.formulas.iter().for_each(|f| {
        let db_path = f.get_db_path();
        let dict_paths = f.get_dict_paths();
        compile_dicts_to_db(dict_paths, db_path).unwrap();
    });

    let state = State::from(&config);
    let state_toml = toml::to_string(&state).unwrap();
    fs::write(PROJECT_DIRS.data_dir.join(".state"), state_toml).unwrap();
}
