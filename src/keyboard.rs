use std::collections::HashSet;

use liushu_core::engine::candidates::Candidate;

/// Result of processing a key event through [`KeyboardProcessor`].
pub enum KeyboardProcessorResponse {
    /// A letter key (a-z) composing pinyin input.
    Composing(u32),
    /// Commit the current composition directly (used by Enter).
    DirectlyCommit,
    /// Commit the first candidate (used by Space).
    Commit,
    /// Toggle ASCII mode (Shift).
    Toggle,
    /// Backspace — delete last pinyin character when composing.
    Backspace,
    /// Not an interesting key; no action needed from the IME side.
    Ignored,
    /// Update preedit string and candidate list.
    Result(String, Vec<Candidate>),
}

#[derive(Debug, Default)]
pub struct KeyboardProcessor {
    handled_keys: HashSet<u32>,
}

impl KeyboardProcessor {
    /// Process a key event by raw keycode and pressed state.
    ///
    /// The caller passes the raw `key` (evdev scancode), whether the key
    /// is pressed, and the current ASCII-mode flag.
    pub fn handle_key(
        &mut self,
        key: u32,
        pressed: bool,
        is_ascii_mode: bool,
    ) -> KeyboardProcessorResponse {
        if is_ascii_mode {
            if pressed && (key == 42 || key == 54) {
                return KeyboardProcessorResponse::Toggle;
            }
            return KeyboardProcessorResponse::Ignored;
        }

        if pressed {
            match key {
                // a-z
                16..=25 | 30..=38 | 44..=50 => {
                    self.handled_keys.insert(key);
                    KeyboardProcessorResponse::Composing(key)
                }
                // Shift (left/right)
                42 | 54 => {
                    self.handled_keys.insert(key);
                    KeyboardProcessorResponse::Toggle
                }
                // Space
                57 => {
                    self.handled_keys.insert(key);
                    KeyboardProcessorResponse::Commit
                }
                // Backspace
                14 => KeyboardProcessorResponse::Backspace,
                // Enter
                28 => KeyboardProcessorResponse::DirectlyCommit,
                _ => KeyboardProcessorResponse::Ignored,
            }
        } else {
            if self.handled_keys.contains(&key) {
                self.handled_keys.remove(&key);
            }
            KeyboardProcessorResponse::Ignored
        }
    }
}
