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
        return Ok(FileNode::File {
            name,
            path: rel,
            size: meta.len(),
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
            FileNode::File { name, size, .. } => {
                out.push_str(prefix);
                out.push_str(connector);
                out.push_str(name);
                out.push_str(&format!("  ({} B)", size));
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
