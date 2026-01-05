use std::collections::HashSet;

use liushu_core::engine::{Engine, InputMethodEngine, candidates::Candidate};
use wayland_client::{WEnum, protocol::wl_keyboard};

use crate::keyboard::KeyboardProcessorResponse;

#[derive(Debug, Default)]
pub struct Composor {
    input: String,
    engine: Engine,
    candidates: Vec<Candidate>,
    handled_keys: HashSet<u32>,
}

impl Composor {
    pub fn with_engine(engine: Engine) -> Self {
        Self {
            engine,
            ..Default::default()
        }
    }

    pub fn process(&mut self, data: KeyboardProcessorResponse) -> KeyboardProcessorResponse {
        match data {
            KeyboardProcessorResponse::Unhandled(wl_keyboard::Event::Key {
                key, state, ..
            }) => match state {
                WEnum::Value(wl_keyboard::KeyState::Pressed) => match key {
                    14 => {
                        if !self.input.is_empty() {
                            self.handled_keys.insert(key);
                            self.input.pop();
                            if let Ok(res) = self.engine.search(&self.input) {
                                self.candidates = res;
                            }
                            KeyboardProcessorResponse::Result(
                                self.input.clone(),
                                self.candidates.clone(),
                            )
                        } else {
                            data
                        }
                    }
                    28 => {
                        if self.input.is_empty() {
                            data
                        } else {
                            self.handled_keys.insert(key);
                            KeyboardProcessorResponse::DirectlyCommit
                        }
                    }
                    _ => data,
                },
                WEnum::Value(wl_keyboard::KeyState::Released) => {
                    if self.handled_keys.contains(&key) {
                        self.handled_keys.remove(&key);
                        KeyboardProcessorResponse::Ignored
                    } else {
                        data
                    }
                }
                _ => data,
            },
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
