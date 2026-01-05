use std::f64::consts::E;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::{collections::HashMap, io::BufRead};

use itertools::Itertools;
use redb::{Database, MultimapTableDefinition, ReadableDatabase, ReadableTable, TableDefinition};

use crate::error::LiushuError;

const INIT_TABLE: TableDefinition<&str, f64> = TableDefinition::new("init_prob");
const TRANS_TABLE: TableDefinition<(&str, &str), f64> = TableDefinition::new("trans_prob");
const EMISS_TABLE: TableDefinition<(&str, &str), f64> = TableDefinition::new("emiss_prob");
const STATES_TABLE: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("states");
const MIN_F: f64 = -3.14e100;

pub fn train_to_db(corpus_file: impl AsRef<Path>, db: &Database) -> Result<(), LiushuError> {
    count_init_prob(corpus_file.as_ref(), db)?;
    count_trans_prob(corpus_file.as_ref(), db)?;
    count_emiss_prob(corpus_file.as_ref(), db)?;
    count_states(db)?;

    Ok(())
}

fn count_init_prob(corpus_file: impl AsRef<Path>, db: &Database) -> Result<(), LiushuError> {
    let mut initial_counts = HashMap::new();
    let mut total_count = 0;

    let file = File::open(corpus_file)?;
    let reader = BufReader::new(file);

    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        let tokens: Vec<&str> = line.split('\t').collect();

        let sentence = tokens[0]
            .split(' ')
            .map(|s| s.to_string())
            .collect::<Vec<String>>();

        let first_char = sentence[0].to_string();
        *initial_counts.entry(first_char).or_insert(0) += 1;
        total_count += 1;

        if idx % 5000 == 0 {
            println!("current init count {}", idx);
        }
    }

    for chunk in initial_counts.iter().chunks(1000).into_iter() {
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(INIT_TABLE)?;
            for (key, &value) in chunk {
                let value = (value as f64 / total_count as f64).log(E);
                table.insert(key.as_str(), value)?;
            }
        }
        write_txn.commit()?;
    }

    Ok(())
}

fn count_trans_prob(corpus_file: impl AsRef<Path>, db: &Database) -> Result<(), LiushuError> {
    let mut trans_map: HashMap<String, HashMap<String, u64>> = HashMap::new();

    let file = File::open(corpus_file)?;
    let reader = BufReader::new(file);

    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        let tokens: Vec<&str> = line.split('\t').collect();

        let mut sentence = tokens[0]
            .split(' ')
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        sentence.push("END".to_string());

        for (word1, word2) in sentence.iter().zip(sentence.iter().skip(1)) {
            let trans_prop = trans_map.entry(word1.to_string()).or_default();
            let next_prob = trans_prop.entry(word2.to_string()).or_insert(0);
            *next_prob += 1;

            if idx % 5000 == 0 {
                println!("current trans count {}", idx);
            }
        }
    }

    for chunk in trans_map.iter().chunks(1000).into_iter() {
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(TRANS_TABLE)?;
            for (pre, value) in chunk {
                let total = value.values().sum::<u64>();
                for (cur, &count) in value {
                    let prob = (count as f64 / total as f64).log(E);
                    table.insert((pre.as_str(), cur.as_str()), prob)?;
                }
            }
        }
        write_txn.commit()?;
    }

    Ok(())
}

fn count_emiss_prob(corpus_file: impl AsRef<Path>, db: &Database) -> Result<(), LiushuError> {
    let mut emit_map: HashMap<String, HashMap<String, u64>> = HashMap::new();

    let file = File::open(corpus_file)?;
    let reader = BufReader::new(file);

    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        let tokens: Vec<&str> = line.split('\t').collect();

        let sentence = tokens[0]
            .split(' ')
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        let pinyin = tokens[1]
            .split(' ')
            .map(|p| p.to_string())
            .collect::<Vec<String>>();
        for (word, py) in sentence.iter().zip(pinyin) {
            let emit_prop = emit_map.entry(word.to_string()).or_default();
            let py_prob = emit_prop.entry(py).or_insert(0);
            *py_prob += 1;
        }

        if idx % 5000 == 0 {
            println!("current emiss count {}", idx);
        }
    }

    for chunk in emit_map.iter().chunks(1000).into_iter() {
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(EMISS_TABLE)?;
            for (word, pinyins) in chunk {
                let total = pinyins.values().sum::<u64>();
                for (py, &count) in pinyins {
                    let prob = (count as f64 / total as f64).log(E);
                    table.insert((word.as_str(), py.as_str()), prob)?;
                }
            }
        }
        write_txn.commit()?;
    }

    Ok(())
}

fn count_states(db: &Database) -> Result<(), LiushuError> {
    let read_txn = db.begin_read()?;
    let write_txn = db.begin_write()?;
    {
        let emission_table = read_txn.open_table(EMISS_TABLE)?;
        let mut states_table = write_txn.open_multimap_table(STATES_TABLE)?;
        for guard in emission_table.iter()? {
            let key = guard?;
            let (word, py) = key.0.value();
            states_table.insert(py, word)?;
        }
    }
    write_txn.commit()?;

    Ok(())
}

pub fn pinyin_to_sentence(py_sequence: &[String], db: &Database) -> Result<String, LiushuError> {
    let read_txn = db.begin_read()?;
    let init_table = read_txn.open_table(INIT_TABLE)?;
    let trans_table = read_txn.open_table(TRANS_TABLE)?;
    let emiss_table = read_txn.open_table(EMISS_TABLE)?;
    let states_table = read_txn.open_multimap_table(STATES_TABLE)?;

    let mut scores = vec![HashMap::new(); py_sequence.len()];
    let mut back_pointers = vec![HashMap::new(); py_sequence.len()];

    // Initialize the first score vector using the initial probabilities
    let first_py = py_sequence[0].clone();
    for result in states_table.get(first_py.as_str())? {
        let guard = result?;
        let state = guard.value();
        let log_init_prob = init_table.get(state)?.map(|x| x.value()).unwrap_or(MIN_F);
        let log_emiss_prob = emiss_table
            .get(&(state, first_py.as_str()))?
            .map(|x| x.value())
            .unwrap_or(MIN_F);
        let score = log_init_prob + log_emiss_prob;
        scores[0].insert(state.to_string(), score);
    }

    // Iterate over the remaining pinyin tokens, computing the score for each possible hanzi
    for (i, py) in py_sequence.iter().skip(1).enumerate() {
        let i = i + 1;
        for result in states_table.get(py.as_str())? {
            let guard = result?;
            let word = guard.value();
            let mut max_score = f64::NEG_INFINITY;
            let mut max_word = String::new();

            // Compute the score for each possible previous hanzi and choose the maximum
            for (prev_word, prev_score) in &scores[i - 1] {
                let log_trans_prob = trans_table
                    .get(&(prev_word.as_str(), word))?
                    .map(|x| x.value())
                    .unwrap_or(MIN_F);
                let score = prev_score + log_trans_prob;
                if score > max_score {
                    max_score = score;
                    max_word = prev_word.to_string();
                }
            }

            // Compute the emission probability for the current hanzi and store the max score and backpointer
            let log_emiss_prob = emiss_table
                .get(&(word, py.as_str()))?
                .map(|x| x.value())
                .unwrap_or(MIN_F);
            let score = max_score + log_emiss_prob;
            scores[i].insert(word.to_string(), score);
            back_pointers[i].insert(word.to_string(), max_word);
        }
    }

    // Determine the most probable hanzi sequence by following the back pointers
    let mut max_final_score = f64::NEG_INFINITY;
    let mut max_final_word = String::new();
    for (word, score) in &scores[py_sequence.len() - 1] {
        if *score > max_final_score {
            max_final_score = *score;
            max_final_word = word.to_string();
        }
    }

    let mut result = max_final_word.clone();
    for i in (0..py_sequence.len() - 1).rev() {
        if let Some(prev_word) = back_pointers[i + 1].get(&max_final_word) {
            result.insert_str(0, prev_word);
            max_final_word = prev_word.clone();
        }
    }

    Ok(result)
}
