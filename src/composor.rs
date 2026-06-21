use liushu_core::engine::{Engine, InputMethodEngine};
use liushu_core::engine::candidates::Candidate;

use crate::keyboard::KeyboardProcessorResponse;

#[derive(Debug, Default)]
pub struct Composor {
    input: String,
    engine: Engine,
    candidates: Vec<Candidate>,
}

impl Composor {
    pub fn with_engine(engine: Engine) -> Self {
        Self {
            engine,
            ..Default::default()
        }
    }

    /// Process a key response and return the action the main loop should take.
    pub fn process(&mut self, data: KeyboardProcessorResponse) -> KeyboardProcessorResponse {
        match data {
            KeyboardProcessorResponse::Backspace => {
                if self.input.is_empty() {
                    return KeyboardProcessorResponse::Ignored;
                }
                self.input.pop();
                if let Ok(res) = self.engine.search(&self.input) {
                    self.candidates = res;
                }
                KeyboardProcessorResponse::Result(self.input.clone(), self.candidates.clone())
            }
            KeyboardProcessorResponse::Composing(spell_key) => {
                let key_str = match spell_key {
                    16 => "q",
                    17 => "w",
                    18 => "e",
                    19 => "r",
                    20 => "t",
                    21 => "y",
                    22 => "u",
                    23 => "i",
                    24 => "o",
                    25 => "p",
                    30 => "a",
                    31 => "s",
                    32 => "d",
                    33 => "f",
                    34 => "g",
                    35 => "h",
                    36 => "j",
                    37 => "k",
                    38 => "l",
                    44 => "z",
                    45 => "x",
                    46 => "c",
                    47 => "v",
                    48 => "b",
                    49 => "n",
                    50 => "m",
                    _ => "",
                };
                self.input.push_str(key_str);
                if let Ok(res) = self.engine.search(&self.input) {
                    self.candidates = res;
                }
                KeyboardProcessorResponse::Result(self.input.clone(), self.candidates.clone())
            }
            _ => data,
        }
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.candidates.clear();
    }
}
