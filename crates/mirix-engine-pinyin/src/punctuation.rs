//! Chinese punctuation state scoped to an input session.
use mirix_engine_protocol::{KeyEvent, PunctuationMode};

#[derive(Default)]
pub(crate) struct Punctuation {
    single_quote_open: bool,
    double_quote_open: bool,
    after_digit: bool,
}

impl Punctuation {
    pub fn end_number(&mut self) {
        self.after_digit = false;
    }

    /// Called only after composition controls (apostrophes, paging and selection).
    pub fn translate(&mut self, event: &KeyEvent, mode: PunctuationMode) -> Option<String> {
        let character = match event.text.as_deref() {
            Some(text) => {
                let mut characters = text.chars();
                characters.next().filter(|_| characters.next().is_none())
            }
            None => char::from_u32(event.keysym).filter(char::is_ascii_graphic),
        };
        let after_digit = self.after_digit;
        self.after_digit = character.is_some_and(|character| character.is_ascii_digit());
        if mode == PunctuationMode::Half {
            return character
                .filter(char::is_ascii_punctuation)
                .map(|c| c.to_string());
        }
        Some(
            match character? {
                '.' if after_digit => return None,
                '.' => "。",
                ',' => "，",
                '!' => "！",
                '?' => "？",
                ':' => "：",
                ';' => "；",
                '(' => "（",
                ')' => "）",
                '[' => "【",
                ']' => "】",
                '<' => "《",
                '>' => "》",
                '/' | '\\' => "、",
                '^' => "……",
                '_' => "——",
                '~' => "～",
                '$' => "￥",
                '\'' => {
                    self.single_quote_open = !self.single_quote_open;
                    if self.single_quote_open { "‘" } else { "’" }
                }
                '"' => {
                    self.double_quote_open = !self.double_quote_open;
                    if self.double_quote_open { "“" } else { "”" }
                }
                _ => return None,
            }
            .to_owned(),
        )
    }
}
