//! Turning free text into safe names for ids, files and folders.
//!
//! Validation stays with each kind of name, because the rules differ on purpose (a skill name, an
//! environment variable, an emoji's code points); what is shared is making a name from text.

/// `name` as lowercase ASCII letters and digits joined by single hyphens, at most `max`
/// characters, with no hyphen at either end. Empty when `name` has no letters or digits.
pub fn slug(name: &str, max: usize) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let cut: String = crate::support::text::clip(&out, max);
    cut.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs_are_lowercase_words_joined_by_single_hyphens() {
        assert_eq!(slug("Security Reviewer!", 60), "security-reviewer");
        assert_eq!(slug("  Weather & Notes!  ", 60), "weather-notes");
        assert_eq!(
            slug("Café — Menu", 60),
            "caf-menu",
            "non-ASCII letters separate words"
        );
        assert_eq!(slug("!!!", 60), "");
    }

    #[test]
    fn a_cut_never_ends_in_a_hyphen() {
        assert_eq!(slug("abc def", 4), "abc");
        assert_eq!(slug("abcdef", 3), "abc");
    }
}
