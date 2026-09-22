use mirix_engine_protocol::KeyEvent;

pub(crate) fn input_letter(event: &KeyEvent) -> Option<char> {
    event
        .text
        .as_deref()
        .filter(|text| text.len() == 1)
        .and_then(|text| text.chars().next())
        .filter(char::is_ascii_lowercase)
}

pub(crate) fn is_apostrophe(event: &KeyEvent) -> bool {
    event.text.as_deref() == Some("'")
}

pub(crate) fn selection_index(event: &KeyEvent) -> Option<usize> {
    let digit = event
        .text
        .as_deref()
        .filter(|text| text.len() == 1)?
        .bytes()
        .next()?;

    digit
        .checked_sub(b'1')
        .filter(|index| *index < 9)
        .map(usize::from)
}
