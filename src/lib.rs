//! tokimo-package-fileparser
//!
//! Pure-Rust unified file parser. You give it an input file and an output
//! directory; the parser dispatches by extension and produces a folder
//! `<output_dir>/<stem>/` containing:
//!
//! - One Markdown file per page / sheet / slide / section, named like
//!   `page-001.md`, `section-002-Sheet1.md`, `section-003-Slide_2.md`, …
//! - An `images/` sub-folder with every embedded image saved in its native
//!   format (`png`, `jpg`, `tif`, `emf`, etc.) when the source contains any.
//!
//! Supported extensions: `.pdf`, `.docx`, `.doc`, `.xlsx`, `.xlsm`, `.xls`,
//! `.pptx`, `.ppt`, `.txt`, `.log`, `.md`, `.markdown`, `.csv`, `.tsv`,
//! `.htm`, `.html`, `.json`.
//!
//! All decoders are pure Rust — no C/C++ libs, no `pdfium`, no system fonts.

mod error;
mod format;
mod office;
mod output;
mod pdf;
mod text;
mod util;

use std::fs;
use std::path::Path;

pub use error::{ParseError, Result};
pub use format::FileKind;
pub use output::{FileNode, ParseOutput};

/// Parse `input` and write the result into `<output_dir>/<input-stem>/`.
///
/// Returns a `ParseOutput` describing the created folder, a JSON-serialisable
/// file tree, and a pretty `tree(1)`-style string.
pub fn parse(input: impl AsRef<Path>, output_dir: impl AsRef<Path>) -> Result<ParseOutput> {
    let input = input.as_ref();
    let output_dir = output_dir.as_ref();

    let kind = FileKind::from_path(input)?;
    let stem = util::input_stem(input);
    let dest = output_dir.join(&stem);
    fs::create_dir_all(&dest)?;

    match kind {
        FileKind::Pdf => {
            pdf::extract_to_dir(input, &dest)?;
        }
        FileKind::Docx | FileKind::Xlsx | FileKind::Pptx | FileKind::Doc | FileKind::Xls | FileKind::Ppt => {
            office::extract_to_dir(input, &dest)?;
        }
        FileKind::Txt => {
            text::txt_to_dir(input, &dest)?;
        }
        FileKind::Markdown => {
            text::markdown_to_dir(input, &dest)?;
        }
        FileKind::Csv => {
            text::csv_to_dir(input, &dest)?;
        }
        #[cfg(feature = "html")]
        FileKind::Html => {
            text::html_to_dir(input, &dest)?;
        }
        FileKind::Json => {
            text::json_to_dir(input, &dest)?;
        }
    }

    Ok(ParseOutput::new(dest)?)
}
