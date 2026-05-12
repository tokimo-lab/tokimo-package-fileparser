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
//! `.pptx`, `.ppt`, `.csv`, `.tsv`.
//!
//! Opt-in features: `.txt`/`.log` (`text`), `.htm`/`.html` (`html`),
//! `.json` (`json`).
//!
//! All decoders are pure Rust — no C/C++ libs, no `pdfium`, no system fonts.

mod archive;
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

/// Default upper bound for archive inputs: 1 GiB.
///
/// Archive extraction is bounded because a single `parse()` call fans out
/// into an arbitrary number of files on disk; this protects callers
/// (typically an LLM Read tool) from accidentally unpacking multi-GB
/// bundles. The limit only applies to inputs detected as archives — PDFs,
/// Office documents, CSVs, etc. are not size-checked.
pub const DEFAULT_MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// Tunables for `parse_with_options`.
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Maximum allowed size, in bytes, for archive inputs. Inputs larger
    /// than this are rejected with [`ParseError::ArchiveTooLarge`] *before*
    /// any extraction starts. Defaults to [`DEFAULT_MAX_ARCHIVE_BYTES`]
    /// (1 GiB). Non-archive inputs ignore this field.
    pub max_archive_bytes: u64,
    /// Optional password for encrypted archives (currently honoured for
    /// `.zip`, `.7z`, `.rar`). `None` (the default) means "no password";
    /// supplying a password for an unencrypted archive is harmless.
    pub archive_password: Option<String>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            max_archive_bytes: DEFAULT_MAX_ARCHIVE_BYTES,
            archive_password: None,
        }
    }
}

/// Parse `input` and write the result into `<output_dir>/<input-stem>/`.
///
/// Returns a `ParseOutput` describing the created folder, a JSON-serialisable
/// file tree, and a pretty `tree(1)`-style string.
///
/// Equivalent to [`parse_with_options`] with [`ParseOptions::default()`],
/// so archive inputs are capped at 1 GiB.
pub fn parse(input: impl AsRef<Path>, output_dir: impl AsRef<Path>) -> Result<ParseOutput> {
    parse_with_options(input, output_dir, &ParseOptions::default())
}

/// Like [`parse`] but lets the caller override the archive size limit
/// (and, in the future, other knobs).
pub fn parse_with_options(
    input: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    opts: &ParseOptions,
) -> Result<ParseOutput> {
    let input = input.as_ref();
    let output_dir = output_dir.as_ref();

    let kind = FileKind::from_path(input)?;

    if kind == FileKind::Archive {
        let size = fs::metadata(input)?.len();
        if size > opts.max_archive_bytes {
            return Err(ParseError::ArchiveTooLarge {
                size,
                limit: opts.max_archive_bytes,
            });
        }
    }

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
        #[cfg(feature = "text")]
        FileKind::Txt => {
            text::txt_to_dir(input, &dest)?;
        }
        FileKind::Csv => {
            text::csv_to_dir(input, &dest)?;
        }
        #[cfg(feature = "html")]
        FileKind::Html => {
            text::html_to_dir(input, &dest)?;
        }
        #[cfg(feature = "json")]
        FileKind::Json => {
            text::json_to_dir(input, &dest)?;
        }
        FileKind::Archive => {
            archive::extract_to_dir(input, &dest, opts.archive_password.as_deref())?;
        }
    }

    Ok(ParseOutput::new(dest)?)
}
