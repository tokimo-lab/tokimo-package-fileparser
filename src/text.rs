use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ParseError, Result};

fn read_text_auto_encoding(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.to_string());
    }
    let (cow, _enc, had_errors) = encoding_rs::GBK.decode(&bytes);
    if had_errors {
        let (cow2, _, had2) = encoding_rs::UTF_16LE.decode(&bytes);
        if !had2 {
            return Ok(cow2.into_owned());
        }
        return Err(ParseError::Encoding(format!(
            "failed to decode {} as UTF-8/GBK/UTF-16",
            path.display()
        )));
    }
    Ok(cow.into_owned())
}

fn write_single(dest: &Path, name: &str, content: String) -> Result<PathBuf> {
    let path = dest.join(name);
    fs::write(&path, content)?;
    Ok(path)
}

pub fn txt_to_dir(path: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let content = read_text_auto_encoding(path)?;
    let title = path.file_name().and_then(|s| s.to_str()).unwrap_or("document");
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n```\n"));
    md.push_str(&content);
    if !content.ends_with('\n') {
        md.push('\n');
    }
    md.push_str("```\n");
    Ok(vec![write_single(dest, "content.md", md)?])
}

pub fn markdown_to_dir(path: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let content = read_text_auto_encoding(path)?;
    Ok(vec![write_single(dest, "content.md", content)?])
}

pub fn csv_to_dir(path: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let content = read_text_auto_encoding(path)?;
    let delimiter = if path
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| e.eq_ignore_ascii_case("tsv"))
        .unwrap_or(false)
    {
        b'\t'
    } else {
        b','
    };

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(content.as_bytes());

    let mut rows: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        rows.push(rec.iter().map(|s| s.to_string()).collect());
    }

    let title = path.file_name().and_then(|s| s.to_str()).unwrap_or("table");
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));

    if rows.is_empty() {
        md.push_str("_(empty table)_\n");
        return Ok(vec![write_single(dest, "content.md", md)?]);
    }

    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let header = &rows[0];
    md.push('|');
    for i in 0..ncols {
        let cell = header.get(i).map(|s| s.as_str()).unwrap_or("");
        md.push(' ');
        md.push_str(&escape_md_cell(cell));
        md.push_str(" |");
    }
    md.push('\n');
    md.push('|');
    for _ in 0..ncols {
        md.push_str(" --- |");
    }
    md.push('\n');
    for row in rows.iter().skip(1) {
        md.push('|');
        for i in 0..ncols {
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            md.push(' ');
            md.push_str(&escape_md_cell(cell));
            md.push_str(" |");
        }
        md.push('\n');
    }
    Ok(vec![write_single(dest, "content.md", md)?])
}

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

#[cfg(feature = "html")]
pub fn html_to_dir(path: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let content = read_text_auto_encoding(path)?;
    let stripped = strip_html_tags(&content);
    let title = path.file_name().and_then(|s| s.to_str()).unwrap_or("document");
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));
    md.push_str(stripped.trim());
    md.push('\n');
    Ok(vec![write_single(dest, "content.md", md)?])
}

#[cfg(feature = "html")]
fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    let mut in_skip: Option<&'static str> = None;

    let mut rest = input;
    while !rest.is_empty() {
        if let Some(tag) = in_skip {
            let close = format!("</{tag}");
            let lower = rest.to_ascii_lowercase();
            if let Some(idx) = lower.find(&close) {
                rest = &rest[idx..];
                if let Some(gt) = rest.find('>') {
                    rest = &rest[gt + 1..];
                } else {
                    rest = "";
                }
                in_skip = None;
                in_tag = false;
                continue;
            } else {
                break;
            }
        }

        let mut chars = rest.char_indices();
        let Some((_, c)) = chars.next() else { break };
        let next_off = chars.next().map(|(i, _)| i).unwrap_or(rest.len());

        if c == '<' {
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("<script") {
                in_skip = Some("script");
            } else if lower.starts_with("<style") {
                in_skip = Some("style");
            }
            in_tag = true;
            rest = &rest[next_off..];
            continue;
        }
        if c == '>' {
            in_tag = false;
            rest = &rest[next_off..];
            continue;
        }
        if !in_tag {
            out.push(c);
        }
        rest = &rest[next_off..];
    }

    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    let mut collapsed = String::with_capacity(out.len());
    let mut blank_lines = 0;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_lines += 1;
            if blank_lines <= 1 {
                collapsed.push('\n');
            }
        } else {
            blank_lines = 0;
            collapsed.push_str(trimmed);
            collapsed.push('\n');
        }
    }
    collapsed
}

pub fn json_to_dir(path: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let content = read_text_auto_encoding(path)?;
    let title = path.file_name().and_then(|s| s.to_str()).unwrap_or("document");
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n```json\n"));
    md.push_str(&content);
    if !content.ends_with('\n') {
        md.push('\n');
    }
    md.push_str("```\n");
    Ok(vec![write_single(dest, "content.md", md)?])
}
