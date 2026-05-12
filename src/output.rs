use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Result of `parse()` — describes what we produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseOutput {
    /// Absolute (or as-passed) path to the per-input folder we created.
    pub dir: PathBuf,
    /// File-tree under `dir`, suitable for JSON serialisation.
    pub files: FileNode,
    /// Pretty `tree(1)`-style rendering of `files`.
    pub tree: String,
}

/// Recursive directory/file tree node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FileNode {
    File {
        name: String,
        path: PathBuf,
        size: u64,
        /// Newline count for text-like files (currently `.md`); `None` for
        /// binary / unsupported extensions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<u64>,
    },
    Dir {
        name: String,
        path: PathBuf,
        children: Vec<FileNode>,
    },
}

impl ParseOutput {
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        let files = build_tree(&dir, &dir)?;
        let tree = render_tree(&files);
        Ok(Self { dir, files, tree })
    }

    /// Serialise the entire output (dir + files + tree) to a JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serialise ParseOutput")
    }

    /// Pretty-printed JSON.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("serialise ParseOutput")
    }
}

fn build_tree(root: &Path, current: &Path) -> std::io::Result<FileNode> {
    let name = current.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let rel = current
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(&name));

    let meta = fs::metadata(current)?;
    if meta.is_file() {
        let lines = count_lines_if_text(current, &name);
        return Ok(FileNode::File {
            name,
            path: rel,
            size: meta.len(),
            lines,
        });
    }

    let mut entries: Vec<_> = fs::read_dir(current)?.filter_map(|r| r.ok()).collect();
    entries.sort_by_key(|e| e.path());

    let mut children = Vec::with_capacity(entries.len());
    for e in entries {
        children.push(build_tree(root, &e.path())?);
    }

    Ok(FileNode::Dir {
        name,
        path: rel,
        children,
    })
}

fn render_tree(node: &FileNode) -> String {
    let mut out = String::new();
    let label = match node {
        FileNode::File { name, .. } => name.clone(),
        FileNode::Dir { name, .. } => name.clone(),
    };
    out.push_str(&label);
    out.push('\n');
    if let FileNode::Dir { children, .. } = node {
        render_children(children, "", &mut out);
    }
    out
}

fn render_children(children: &[FileNode], prefix: &str, out: &mut String) {
    let n = children.len();
    for (i, child) in children.iter().enumerate() {
        let last = i + 1 == n;
        let connector = if last { "└── " } else { "├── " };
        let next_prefix = if last { "    " } else { "│   " };
        match child {
            FileNode::File { name, size, lines, .. } => {
                out.push_str(prefix);
                out.push_str(connector);
                out.push_str(name);
                out.push_str("  (");
                out.push_str(&format_size(*size));
                if let Some(n) = lines {
                    out.push_str(&format!(", {n} lines"));
                }
                out.push(')');
                out.push('\n');
            }
            FileNode::Dir { name, children, .. } => {
                out.push_str(prefix);
                out.push_str(connector);
                out.push_str(name);
                out.push('/');
                out.push('\n');
                let combined = format!("{prefix}{next_prefix}");
                render_children(children, &combined, out);
            }
        }
    }
}

/// Human-readable byte size: `B` below 1 KiB, `KB` below 1 MiB, else `MB`.
/// Uses 1024-based units (binary) and one decimal place for KB/MB.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}

/// Count newlines for known text outputs (currently `.md`). Returns
/// `None` for unsupported extensions or unreadable files (caller falls
/// back to size-only rendering).
fn count_lines_if_text(path: &Path, name: &str) -> Option<u64> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    if ext != "md" {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    Some(content.lines().count() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_thresholds() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(15922), "15.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }
}
