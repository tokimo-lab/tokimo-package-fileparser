use std::path::Path;

pub fn sanitize_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    let trimmed = out.trim_matches(|c: char| c == '_' || c == '.').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else if trimmed.len() > 60 {
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
