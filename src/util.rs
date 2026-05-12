use std::path::Path;

pub fn sanitize_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        // Replace only filesystem-illegal characters (Windows is the strictest)
        // plus control characters. Preserve whitespace and Unicode (CJK,
        // emoji, etc.) since the three target OSes (Windows, macOS, Linux)
        // all accept them in filenames.
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    let trimmed = out
        .trim_matches(|c: char| c == '_' || c == '.' || c.is_whitespace())
        .to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else if trimmed.chars().count() > 60 {
        trimmed.chars().take(60).collect()
    } else {
        trimmed
    }
}

pub fn input_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(sanitize_filename)
        .unwrap_or_else(|| "output".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_whitespace() {
        assert_eq!(sanitize_filename("1 (4)"), "1 (4)");
        assert_eq!(sanitize_filename("My Document"), "My Document");
    }

    #[test]
    fn replaces_illegal_chars() {
        assert_eq!(sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
    }

    #[test]
    fn replaces_control_chars() {
        // \t (0x09) and \n (0x0A) are classified as control in Rust's
        // char::is_control, so they are replaced with underscores.
        assert_eq!(sanitize_filename("a\tb\nc"), "a_b_c");
    }

    #[test]
    fn trims_outer_whitespace_and_dots() {
        assert_eq!(sanitize_filename("  hello  "), "hello");
        assert_eq!(sanitize_filename("..foo.."), "foo");
    }

    #[test]
    fn truncates_at_char_boundary() {
        let long_cjk = "中".repeat(100);
        let out = sanitize_filename(&long_cjk);
        assert_eq!(out.chars().count(), 60);
    }

    #[test]
    fn empty_after_sanitize_returns_untitled() {
        assert_eq!(sanitize_filename("..."), "untitled");
        assert_eq!(sanitize_filename(""), "untitled");
        assert_eq!(sanitize_filename("///"), "untitled");
    }

    #[test]
    fn preserves_unicode() {
        assert_eq!(sanitize_filename("文档"), "文档");
        assert_eq!(sanitize_filename("résumé"), "résumé");
    }
}
