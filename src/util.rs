//! Small shared utilities.

/// Whitespace-split a command string into tokens, respecting `"double quotes"`.
///
/// Quotes group a token and are removed from the output. Used to parse the
/// `--launch` and `--transcribe-cmd` command strings.
///
/// **Limitations (by design):** there is no escape mechanism — a literal
/// double-quote cannot be included in an argument, and backslashes are taken
/// literally (so Windows paths like `C:\models\ggml.bin` pass through unchanged).
/// An argument that itself contains spaces must be wrapped in double quotes. If
/// you need full control, build the argument vector yourself rather than a
/// command string.
///
/// ```
/// assert_eq!(
///     framewatch::tokenize("prog --flag \"two words\" C:\\path\\file"),
///     vec!["prog", "--flag", "two words", "C:\\path\\file"]
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
