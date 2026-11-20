//! Small text helpers.

/// At most the first `max` characters of `text` (characters, not bytes, so a cut never splits
/// one).
pub fn clip(text: impl AsRef<str>, max: usize) -> String {
    text.as_ref().chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::clip;

    #[test]
    fn clips_by_characters() {
        assert_eq!(clip("héllo", 2), "hé");
        assert_eq!(clip(String::from("abc"), 10), "abc");
        assert_eq!(clip("", 3), "");
    }
}
