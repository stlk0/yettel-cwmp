//! Sanitize untrusted strings before terminal display.
/// Identify controls and direction overrides that can mislead terminal readers.
pub fn unsafe_terminal_control(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// Replace controls and direction overrides with U+FFFD so untrusted text cannot move the
/// cursor, change colors or reorder what the reader sees.
pub fn safe(text: &str) -> String {
    text.chars()
        .map(|character| {
            if unsafe_terminal_control(character) {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_marks_and_terminal_controls_are_replaced() {
        for character in [
            '\u{001b}', '\u{0085}', '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202e}',
            '\u{2066}', '\u{2069}',
        ] {
            assert!(unsafe_terminal_control(character));
            assert_eq!(safe(&format!("č{character}ć")), "č�ć");
        }
        assert_eq!(safe("Čačak – korisničko ime"), "Čačak – korisničko ime");
    }
}
