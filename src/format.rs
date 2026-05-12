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
}

impl FileKind {
    pub fn from_path(path: &Path) -> Result<Self> {
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
