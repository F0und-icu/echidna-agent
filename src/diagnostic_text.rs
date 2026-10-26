//! Pure control-character projection shared by diagnostics and terminal views.

pub(crate) fn escaped_text(text: &str) -> String {
    let mut rendered = String::new();
    for character in text.chars() {
        escape_character(character, |character| rendered.push(character));
    }
    rendered
}

pub(crate) fn escape_character(character: char, mut emit: impl FnMut(char)) {
    if character.is_control() && !matches!(character, '\n' | '\t') {
        character.escape_default().for_each(emit);
    } else {
        emit(character);
    }
}
