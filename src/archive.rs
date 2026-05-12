//! Archive extraction adapter.
//!
//! Delegates to the `tokimo-universal-archiver` crate (lib name `archiver`)
//! to fully extract any supported archive into `<dest>/` so that
//! `ParseOutput::new(dest)` can walk the resulting tree.

use std::path::Path;

use crate::error::{ParseError, Result};

/// Fully extract `input` (a supported archive) into `dest`.
///
/// All entries are written under `dest/`; the existing tree-builder in
/// `output.rs` then produces the `files` / `tree` view exactly as it does
/// for any other input type. `password`, when `Some`, is forwarded to the
/// archiver for encrypted ZIP / 7Z / RAR inputs.
pub fn extract_to_dir(input: &Path, dest: &Path, password: Option<&str>) -> Result<()> {
    let opts = password.map(|p| archiver::types::OpenOptions {
        password: Some(p.to_string()),
    });
    archiver::extract_all(input, dest, opts.as_ref()).map_err(map_err)
}

/// Return `true` if `path`'s name matches one of the archive formats the
/// universal-archiver crate understands. Cheap — only inspects the filename.
pub fn is_archive(path: &Path) -> bool {
    archiver::detect(path).is_ok()
}

fn map_err(e: archiver::error::ArchiveError) -> ParseError {
    match e {
        archiver::error::ArchiveError::Io(io) => ParseError::Io(io),
        other => ParseError::Archive(other.to_string()),
    }
}
