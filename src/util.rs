//! Small shared utilities.

/// Whitespace-split a command string into tokens, respecting `"double quotes"`.
///
/// Quotes group a token and are removed from the output. Used to parse the
/// `--launch` and `--transcribe-cmd` command strings.
///
/// ```
/// assert_eq!(
///     framewatch::tokenize("prog --flag \"two words\""),
///     vec!["prog", "--flag", "two words"]
/// );
/// ```
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_respects_quotes_and_whitespace() {
        assert_eq!(
            tokenize("game.exe --freecam --pos \"1 2 3\""),
            vec!["game.exe", "--freecam", "--pos", "1 2 3"]
        );
        assert_eq!(tokenize("   a   b  "), vec!["a", "b"]);
        assert!(tokenize("   ").is_empty());
    }
}
