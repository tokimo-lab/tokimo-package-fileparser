use std::path::Path;

use crate::error::{ParseError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Doc,
    Xls,
    Ppt,
    #[cfg(feature = "text")]
    Txt,
    Csv,
    #[cfg(feature = "html")]
    Html,
    #[cfg(feature = "json")]
    Json,
    /// Any archive supported by `tokimo-universal-archiver`
    /// (`.zip`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tbz2`, `.tar.xz`/`.txz`,
    /// `.tar.zst`/`.tzst`, `.7z`, `.rar`, `.gz`, `.bz2`, `.xz`, `.zst`).
    Archive,
}

impl FileKind {
    pub fn from_path(path: &Path) -> Result<Self> {
        // Archive formats use compound extensions (e.g. `.tar.gz`) so try the
        // archive detector first — it understands those, and it covers
        // single-extension archives (`.zip`, `.7z`, …) too.
        if crate::archive::is_archive(path) {
            return Ok(FileKind::Archive);
        }

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ParseError::MissingExtension(path.display().to_string()))?
            .to_ascii_lowercase();

        Ok(match ext.as_str() {
            "pdf" => FileKind::Pdf,
            "docx" => FileKind::Docx,
            "xlsx" | "xlsm" => FileKind::Xlsx,
            "pptx" => FileKind::Pptx,
            "doc" => FileKind::Doc,
            "xls" => FileKind::Xls,
            "ppt" => FileKind::Ppt,
            #[cfg(feature = "text")]
            "txt" | "log" => FileKind::Txt,
            #[cfg(not(feature = "text"))]
            "txt" | "log" => {
                return Err(ParseError::UnsupportedExtension(format!(
                    "{ext} (text support is gated behind the `text` cargo feature)"
                )));
            }
            "csv" | "tsv" => FileKind::Csv,
            #[cfg(feature = "html")]
            "htm" | "html" => FileKind::Html,
            #[cfg(not(feature = "html"))]
            "htm" | "html" => {
                return Err(ParseError::UnsupportedExtension(format!(
                    "{ext} (HTML support is gated behind the `html` cargo feature)"
                )));
            }
            #[cfg(feature = "json")]
            "json" => FileKind::Json,
            #[cfg(not(feature = "json"))]
            "json" => {
                return Err(ParseError::UnsupportedExtension(format!(
                    "{ext} (JSON support is gated behind the `json` cargo feature)"
                )));
            }
            other => return Err(ParseError::UnsupportedExtension(other.to_string())),
        })
    }
}
