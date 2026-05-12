use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use office_oxide::Document;
use office_oxide::ir::{DocumentIR, Element, Image, ImageFormat, Section};

use crate::error::{ParseError, Result};
use crate::util::sanitize_filename;

/// Per-source-container grouping kind, derived from the input extension.
/// Drives the `images/<container>/` subdirectory layout.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OfficeKind {
    /// PPTX → `images/slide-NNN/`
    Pptx,
    /// XLSX/XLSM → `images/sheet-NNN[-name]/`
    Xlsx,
    /// DOCX and legacy formats → flat `images/`
    Flat,
}

impl OfficeKind {
    fn from_path(p: &Path) -> Self {
        match p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("pptx") => OfficeKind::Pptx,
            Some("xlsx") | Some("xlsm") => OfficeKind::Xlsx,
            _ => OfficeKind::Flat,
        }
    }
}

pub fn extract_to_dir(input: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let doc = Document::open(input)?;
    let ir = doc.to_ir();
    let kind = OfficeKind::from_path(input);
    let images_dir = dest.join("images");
    let mut written: Vec<PathBuf> = Vec::new();
    let mut image_counter: u32 = 0;

    let single_section = ir.sections.len() == 1;

    for (i, section) in ir.sections.iter().enumerate() {
        let mut section_clone = section.clone();
        let bucket = section_bucket(kind, i, section_clone.title.as_deref());
        let saved = save_section_images(&mut section_clone, &images_dir, bucket.as_deref(), &mut image_counter)?;

        // For long flowing content (typical DOCX) one Section may carry the
        // whole document. Try to split it further so a 200-page Word doesn't
        // become a single .md file.
        let chunks = split_long_section(&section_clone);

        let section_prefix = if single_section {
            String::new()
        } else {
            let title_slug = section_clone
                .title
                .as_deref()
                .map(sanitize_filename)
                .filter(|s| !s.is_empty())
                .map(|s| format!("-{s}"))
                .unwrap_or_default();
            format!("section-{:03}{}-", i + 1, title_slug)
        };

        for (j, chunk) in chunks.iter().enumerate() {
            let chunk_label = match &chunk.label {
                ChunkLabel::Single => {
                    let title_slug = section_clone
                        .title
                        .as_deref()
                        .map(sanitize_filename)
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("-{s}"))
                        .unwrap_or_default();
                    if single_section {
                        format!("section-001{title_slug}")
                    } else {
                        format!("section-{:03}{title_slug}", i + 1)
                    }
                }
                ChunkLabel::Page => format!("{section_prefix}page-{:03}", j + 1),
                ChunkLabel::Chapter(title) => {
                    let slug = sanitize_filename(title);
                    let suffix = if slug.is_empty() {
                        String::new()
                    } else {
                        format!("-{slug}")
                    };
                    format!("{section_prefix}chapter-{:03}{suffix}", j + 1)
                }
            };
            let filename = format!("{chunk_label}.md");
            let path = dest.join(&filename);

            let chunk_section = Section {
                title: if j == 0 {
                    chunk.title_override.clone().or_else(|| section_clone.title.clone())
                } else {
                    chunk.title_override.clone()
                },
                elements: chunk.elements.clone(),
                ..section_clone.clone()
            };
            let single = DocumentIR {
                metadata: ir.metadata.clone(),
                sections: vec![chunk_section],
            };
            let mut md = single.to_markdown();
            if !md.ends_with('\n') {
                md.push('\n');
            }
            if j == chunks.len() - 1 && !saved.is_empty() {
                md.push_str("\n## Extracted images\n\n");
                for rel in &saved {
                    md.push_str(&format!("![]({rel})\n"));
                }
            }
            fs::write(&path, md)?;
            written.push(path);
        }
    }

    // Fallback: many docx/xlsx/pptx images are not surfaced by office_oxide
    // (it skips drawing/anchor/inline). Also OOXML zips can carry non-image
    // media (audio/video) and embedded objects (xlsx-in-docx, OLE blobs, etc.).
    // Pull everything under `*/media/` and `*/embeddings/` directly from the zip.
    let assets = extract_zip_assets(input, dest, kind, &mut image_counter).unwrap_or_default();
    let images: Vec<&ExtractedAsset> = assets.iter().filter(|a| a.kind == AssetKind::Image).collect();
    let media: Vec<&ExtractedAsset> = assets.iter().filter(|a| a.kind == AssetKind::Media).collect();
    let embeds: Vec<&ExtractedAsset> = assets.iter().filter(|a| a.kind == AssetKind::Embedding).collect();

    if !images.is_empty() {
        let path = dest.join("images.md");
        let mut md = String::from("# Embedded images\n\n");
        for a in &images {
            md.push_str(&format!("![]({})\n\n", a.rel));
        }
        fs::write(&path, md)?;
        written.push(path);
    }
    if !media.is_empty() {
        let path = dest.join("media.md");
        let mut md = String::from("# Embedded media (audio/video/other)\n\n");
        for a in &media {
            md.push_str(&format!("- [{}]({})\n", a.original, a.rel));
        }
        fs::write(&path, md)?;
        written.push(path);
    }
    if !embeds.is_empty() {
        let path = dest.join("embeddings.md");
        let mut md = String::from("# Embedded objects\n\n");
        for a in &embeds {
            md.push_str(&format!("- [{}]({})\n", a.original, a.rel));
        }
        fs::write(&path, md)?;
        written.push(path);
    }

    Ok(written)
}

#[derive(PartialEq, Eq)]
enum AssetKind {
    Image,
    Media,
    Embedding,
}

struct ExtractedAsset {
    kind: AssetKind,
    rel: String,
    original: String,
}

fn extract_zip_assets(
    input: &Path,
    dest: &Path,
    kind: OfficeKind,
    image_counter: &mut u32,
) -> Result<Vec<ExtractedAsset>> {
    let file = fs::File::open(input)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| ParseError::Zip(e.to_string()))?;
    let mut out: Vec<ExtractedAsset> = Vec::new();
    let mut media_counter: u32 = 0;
    let mut embed_counter: u32 = 0;
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    // First pass: build a media-path → container map by reading `.rels`
    // sidecars. `media_buckets` keys are zip-internal absolute paths (e.g.
    // `ppt/media/image1.png`); values are subdirectory names (e.g.
    // `slide-001`). Images not referenced by any slide / sheet rel fall
    // through to the flat `images/` directory.
    let media_buckets = build_zip_media_buckets(&mut zip, &names, kind);

    for name in names {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with('/') {
            continue;
        }
        let in_media = lower.contains("/media/");
        let in_embed = lower.contains("/embeddings/") || lower.contains("/oleobject");
        if !in_media && !in_embed {
            continue;
        }

        let mut entry = match zip.by_name(&name) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() || buf.is_empty() {
            continue;
        }
        let original = Path::new(&name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&name)
            .to_string();
        let ext = Path::new(&name)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .or_else(|| sniff_extension(&buf).map(|s| s.to_string()))
            .unwrap_or_else(|| "bin".to_string());

        let (asset_kind, sub, filename) = if in_embed {
            embed_counter += 1;
            (
                AssetKind::Embedding,
                "embeddings".to_string(),
                format!("embed-{:03}-{}", embed_counter, sanitize_filename(&original)),
            )
        } else if is_image_ext(&ext) {
            *image_counter += 1;
            let bucket = media_buckets.get(&name).cloned();
            let sub = match (kind, bucket) {
                (OfficeKind::Flat, _) | (_, None) => "images".to_string(),
                (_, Some(b)) => format!("images/{b}"),
            };
            (AssetKind::Image, sub, format!("img-{:03}.{ext}", *image_counter))
        } else {
            media_counter += 1;
            (
                AssetKind::Media,
                "media".to_string(),
                format!("media-{:03}.{ext}", media_counter),
            )
        };
        let sub_dir = dest.join(&sub);
        if !sub_dir.exists() {
            fs::create_dir_all(&sub_dir)?;
        }
        fs::write(sub_dir.join(&filename), &buf)?;
        out.push(ExtractedAsset {
            kind: asset_kind,
            rel: format!("{sub}/{filename}"),
            original,
        });
    }
    Ok(out)
}

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "emf" | "wmf" | "svg"
    )
}

#[derive(Clone)]
enum ChunkLabel {
    Single,
    Page,
    Chapter(String),
}

struct Chunk {
    label: ChunkLabel,
    title_override: Option<String>,
    elements: Vec<Element>,
}

/// Maximum number of top-level elements before we force-split a chunk so a
/// huge unstructured DOCX still produces multiple files instead of one giant one.
const MAX_ELEMENTS_PER_CHUNK: usize = 200;

fn split_long_section(section: &Section) -> Vec<Chunk> {
    // office_oxide maps both real `<w:pageBreak>` and `<w:br w:type="page"/>`
    // (the common Word page break) into `Element::PageBreak` *or*
    // `Element::ThematicBreak` depending on version – treat both as page
    // boundaries so a long flowing DOCX gets cut into pages.
    let is_page_boundary = |e: &Element| matches!(e, Element::PageBreak | Element::ThematicBreak);
    let has_page_break = section.elements.iter().any(is_page_boundary);

    if has_page_break {
        let mut chunks: Vec<Chunk> = Vec::new();
        let mut current: Vec<Element> = Vec::new();
        for el in &section.elements {
            if is_page_boundary(el) {
                if !current.is_empty() {
                    chunks.push(Chunk {
                        label: ChunkLabel::Page,
                        title_override: None,
                        elements: std::mem::take(&mut current),
                    });
                }
            } else {
                current.push(el.clone());
            }
        }
        if !current.is_empty() {
            chunks.push(Chunk {
                label: ChunkLabel::Page,
                title_override: None,
                elements: current,
            });
        }
        if chunks.len() >= 2 {
            return chunks;
        }
    }

    // Try splitting by top-level H1.
    let h1_count = section
        .elements
        .iter()
        .filter(|e| matches!(e, Element::Heading(h) if h.level == 1))
        .count();
    if h1_count >= 2 {
        let mut chunks: Vec<Chunk> = Vec::new();
        let mut current: Vec<Element> = Vec::new();
        let mut current_title: Option<String> = None;
        for el in &section.elements {
            if let Element::Heading(h) = el {
                if h.level == 1 && !current.is_empty() {
                    chunks.push(Chunk {
                        label: ChunkLabel::Chapter(current_title.clone().unwrap_or_default()),
                        title_override: current_title.take(),
                        elements: std::mem::take(&mut current),
                    });
                }
                if h.level == 1 {
                    current_title = Some(heading_plain_text(h));
                }
            }
            current.push(el.clone());
        }
        if !current.is_empty() {
            chunks.push(Chunk {
                label: ChunkLabel::Chapter(current_title.clone().unwrap_or_default()),
                title_override: current_title,
                elements: current,
            });
        }
        return chunks;
    }

    // Fall back to fixed-size chunking only if the section is really big.
    if section.elements.len() > MAX_ELEMENTS_PER_CHUNK {
        let mut chunks: Vec<Chunk> = Vec::new();
        for chunk in section.elements.chunks(MAX_ELEMENTS_PER_CHUNK) {
            chunks.push(Chunk {
                label: ChunkLabel::Page,
                title_override: None,
                elements: chunk.to_vec(),
            });
        }
        return chunks;
    }

    vec![Chunk {
        label: ChunkLabel::Single,
        title_override: None,
        elements: section.elements.clone(),
    }]
}

fn heading_plain_text(h: &office_oxide::ir::Heading) -> String {
    let mut s = String::new();
    for ic in &h.content {
        if let office_oxide::ir::InlineContent::Text(t) = ic {
            s.push_str(&t.text);
        }
    }
    s.trim().to_string()
}

fn save_section_images(
    section: &mut Section,
    images_dir: &Path,
    bucket: Option<&str>,
    counter: &mut u32,
) -> Result<Vec<String>> {
    let mut saved: Vec<String> = Vec::new();
    walk_elements(&mut section.elements, &mut |img| {
        if let Some(rel) = persist_image(img, images_dir, bucket, counter)? {
            saved.push(rel.clone());
            img.alt_text = Some(rel);
        }
        Ok(())
    })?;
    Ok(saved)
}

fn walk_elements<F>(elements: &mut [Element], f: &mut F) -> Result<()>
where
    F: FnMut(&mut Image) -> Result<()>,
{
    for el in elements.iter_mut() {
        walk_element(el, f)?;
    }
    Ok(())
}

fn walk_element<F>(el: &mut Element, f: &mut F) -> Result<()>
where
    F: FnMut(&mut Image) -> Result<()>,
{
    match el {
        Element::Image(img) => f(img)?,
        Element::List(list) => {
            for item in list.items.iter_mut() {
                walk_elements(&mut item.content, f)?;
                if let Some(nested) = item.nested.as_mut() {
                    for sub in nested.items.iter_mut() {
                        walk_elements(&mut sub.content, f)?;
                    }
                }
            }
        }
        Element::Table(t) => {
            for row in t.rows.iter_mut() {
                for cell in row.cells.iter_mut() {
                    walk_elements(&mut cell.content, f)?;
                }
            }
        }
        Element::TextBox(tb) => walk_elements(&mut tb.content, f)?,
        Element::Footnote(note) | Element::Endnote(note) => walk_elements(&mut note.content, f)?,
        _ => {}
    }
    Ok(())
}

fn persist_image(img: &Image, images_dir: &Path, bucket: Option<&str>, counter: &mut u32) -> Result<Option<String>> {
    let Some(bytes) = img.data.as_ref() else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    let target_dir: PathBuf = match bucket {
        Some(b) => images_dir.join(b),
        None => images_dir.to_path_buf(),
    };
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir)?;
    }
    *counter += 1;
    let ext: &str = match &img.format {
        Some(fmt) => fmt.extension(),
        None => sniff_extension(bytes).unwrap_or("bin"),
    };
    let filename = format!("img-{:03}.{ext}", *counter);
    let abs = target_dir.join(&filename);
    fs::write(&abs, bytes)?;
    let rel = match bucket {
        Some(b) => format!("images/{b}/{filename}"),
        None => format!("images/{filename}"),
    };
    Ok(Some(rel))
}

fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("tif")
    } else {
        None
    }
}

#[allow(dead_code)]
fn _format_to_ext(f: ImageFormat) -> &'static str {
    f.extension()
}

/// Returns the per-section image subdirectory (relative to `images/`) for a
/// given OfficeKind. Returns `None` for DOCX-style flat layouts.
fn section_bucket(kind: OfficeKind, index: usize, title: Option<&str>) -> Option<String> {
    match kind {
        OfficeKind::Flat => None,
        OfficeKind::Pptx => Some(format!("slide-{:03}", index + 1)),
        OfficeKind::Xlsx => {
            let slug = title
                .map(sanitize_filename)
                .filter(|s| !s.is_empty() && s != "untitled");
            match slug {
                Some(s) => Some(format!("sheet-{:03}-{s}", index + 1)),
                None => Some(format!("sheet-{:03}", index + 1)),
            }
        }
    }
}

/// Parse the relevant `*.rels` sidecars in the OOXML zip to map each
/// referenced media file (by zip-internal absolute path) to a slide / sheet
/// container subdirectory. Returns an empty map for `OfficeKind::Flat`
/// (DOCX) or on any parse failure.
fn build_zip_media_buckets(
    zip: &mut zip::ZipArchive<fs::File>,
    names: &[String],
    kind: OfficeKind,
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    match kind {
        OfficeKind::Flat => out,
        OfficeKind::Pptx => {
            // ppt/slides/_rels/slideN.xml.rels — slide rels reference media
            // directly via `Target="../media/imageN.png"`.
            for name in names {
                let lower = name.to_ascii_lowercase();
                if !(lower.starts_with("ppt/slides/_rels/") && lower.ends_with(".xml.rels") && lower.contains("/slide"))
                {
                    continue;
                }
                let Some(slide_num) = parse_numeric_suffix(&name_stem_no_xml_rels(name), "slide") else {
                    continue;
                };
                let bucket = format!("slide-{:03}", slide_num);
                let Some(xml) = read_zip_text(zip, name) else { continue };
                for target in extract_targets(&xml) {
                    // base = parent of the file the rels describes,
                    // i.e. `ppt/slides/`.
                    if let Some(abs) = resolve_rel_target("ppt/slides/", &target)
                        && is_media_path(&abs)
                    {
                        out.entry(abs).or_insert_with(|| bucket.clone());
                    }
                }
            }
            out
        }
        OfficeKind::Xlsx => {
            // xlsx is a two-hop chain: sheet rel → drawing.xml → drawing rel → media.
            // First, build sheet_num → drawing_path map from sheet rels.
            // Then, for each drawing rel, propagate the sheet container.
            // sheet name lookup is intentionally skipped here (handled via
            // office_oxide Section title in the IR walk).
            let mut drawing_to_sheet: HashMap<String, u32> = HashMap::new();
            for name in names {
                let lower = name.to_ascii_lowercase();
                if !(lower.starts_with("xl/worksheets/_rels/") && lower.ends_with(".xml.rels")) {
                    continue;
                }
                let Some(sheet_num) = parse_numeric_suffix(&name_stem_no_xml_rels(name), "sheet") else {
                    continue;
                };
                let Some(xml) = read_zip_text(zip, name) else { continue };
                for target in extract_targets(&xml) {
                    if let Some(abs) = resolve_rel_target("xl/worksheets/", &target) {
                        let abs_lower = abs.to_ascii_lowercase();
                        if abs_lower.starts_with("xl/drawings/") && abs_lower.ends_with(".xml") {
                            drawing_to_sheet.entry(abs).or_insert(sheet_num);
                        } else if is_media_path(&abs) {
                            // Rare: direct image hyperlink on a sheet.
                            out.entry(abs).or_insert_with(|| format!("sheet-{:03}", sheet_num));
                        }
                    }
                }
            }
            // Now resolve drawing rels.
            for name in names {
                let lower = name.to_ascii_lowercase();
                if !(lower.starts_with("xl/drawings/_rels/") && lower.ends_with(".xml.rels")) {
                    continue;
                }
                // Strip `_rels/` to get the drawing path the rels describes.
                let drawing_path = name
                    .replacen("xl/drawings/_rels/", "xl/drawings/", 1)
                    .trim_end_matches(".rels")
                    .to_string();
                let Some(&sheet_num) = drawing_to_sheet.get(&drawing_path) else {
                    continue;
                };
                let bucket = format!("sheet-{:03}", sheet_num);
                let Some(xml) = read_zip_text(zip, name) else { continue };
                for target in extract_targets(&xml) {
                    if let Some(abs) = resolve_rel_target("xl/drawings/", &target)
                        && is_media_path(&abs)
                    {
                        out.entry(abs).or_insert_with(|| bucket.clone());
                    }
                }
            }
            out
        }
    }
}

fn read_zip_text(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Option<String> {
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    Some(buf)
}

/// Strip the trailing `.xml.rels` so callers can run `parse_numeric_suffix`
/// over the bare file stem (e.g. `slide12` or `sheet3`).
fn name_stem_no_xml_rels(name: &str) -> String {
    let file = Path::new(name).file_name().and_then(|s| s.to_str()).unwrap_or(name);
    file.trim_end_matches(".rels").trim_end_matches(".xml").to_string()
}

/// Returns the trailing `u32` from a stem matching `<prefix>NN`.
fn parse_numeric_suffix(stem: &str, prefix: &str) -> Option<u32> {
    let rest = stem.strip_prefix(prefix)?;
    rest.parse::<u32>().ok()
}

/// Pull every `Target="..."` attribute value out of a `*.rels` document.
/// Hand-rolled to avoid pulling in a full XML parser dependency; rels files
/// are flat one-element-per-line manifests so this is robust enough.
fn extract_targets(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = xml.as_bytes();
    let needle = b"Target=\"";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            i += needle.len();
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            if i <= bytes.len() {
                out.push(String::from_utf8_lossy(&bytes[start..i]).into_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Resolve a `Target` attribute (which is relative to the *described* file's
/// parent directory) into a zip-internal absolute path. Handles `../`
/// segments and forward-slash separators.
fn resolve_rel_target(base_dir: &str, target: &str) -> Option<String> {
    // Targets starting with `/` are zip-absolute per OOXML spec.
    let combined = if let Some(rest) = target.strip_prefix('/') {
        rest.to_string()
    } else {
        format!("{base_dir}{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("/")) }
}

fn is_media_path(p: &str) -> bool {
    p.to_ascii_lowercase().contains("/media/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_rel_target_with_parent_segments() {
        assert_eq!(
            resolve_rel_target("ppt/slides/", "../media/image1.png").as_deref(),
            Some("ppt/media/image1.png")
        );
        assert_eq!(
            resolve_rel_target("xl/worksheets/", "../drawings/drawing1.xml").as_deref(),
            Some("xl/drawings/drawing1.xml")
        );
        assert_eq!(
            resolve_rel_target("xl/drawings/", "../media/image2.jpeg").as_deref(),
            Some("xl/media/image2.jpeg")
        );
        assert_eq!(
            resolve_rel_target("ppt/slides/", "/ppt/media/image1.png").as_deref(),
            Some("ppt/media/image1.png")
        );
    }

    #[test]
    fn extracts_target_attribute_values() {
        let xml = r#"<?xml version="1.0"?>
            <Relationships xmlns="x">
              <Relationship Id="rId1" Type="img" Target="../media/image1.png"/>
              <Relationship Id="rId2" Type="img" Target="../media/image2.jpeg"/>
            </Relationships>"#;
        let got = extract_targets(xml);
        assert_eq!(got, vec!["../media/image1.png", "../media/image2.jpeg"]);
    }

    #[test]
    fn parses_numeric_suffix() {
        assert_eq!(parse_numeric_suffix("slide12", "slide"), Some(12));
        assert_eq!(parse_numeric_suffix("sheet3", "sheet"), Some(3));
        assert_eq!(parse_numeric_suffix("notes1", "slide"), None);
    }

    #[test]
    fn section_bucket_layout() {
        assert_eq!(section_bucket(OfficeKind::Flat, 0, Some("anything")), None);
        assert_eq!(section_bucket(OfficeKind::Pptx, 0, None).as_deref(), Some("slide-001"));
        assert_eq!(
            section_bucket(OfficeKind::Pptx, 11, Some("Title")).as_deref(),
            Some("slide-012")
        );
        assert_eq!(
            section_bucket(OfficeKind::Xlsx, 0, Some("Sales 2024")).as_deref(),
            Some("sheet-001-Sales 2024")
        );
        assert_eq!(section_bucket(OfficeKind::Xlsx, 2, None).as_deref(), Some("sheet-003"));
        assert_eq!(
            section_bucket(OfficeKind::Xlsx, 0, Some("")).as_deref(),
            Some("sheet-001")
        );
    }

    #[test]
    fn builds_pptx_media_buckets_from_synthetic_zip() {
        use std::io::Write;
        // Build a minimal pptx-shaped zip with two slide rels each
        // pointing at a different media file.
        let mut zip_bytes: Vec<u8> = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut w = zip::ZipWriter::new(cursor);
            let opts: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            w.start_file("ppt/slides/_rels/slide1.xml.rels", opts).unwrap();
            w.write_all(
                br#"<?xml version="1.0"?><Relationships>
                    <Relationship Id="r1" Target="../media/image1.png"/>
                </Relationships>"#,
            )
            .unwrap();
            w.start_file("ppt/slides/_rels/slide2.xml.rels", opts).unwrap();
            w.write_all(
                br#"<?xml version="1.0"?><Relationships>
                    <Relationship Id="r1" Target="../media/image2.jpeg"/>
                </Relationships>"#,
            )
            .unwrap();
            w.start_file("ppt/media/image1.png", opts).unwrap();
            w.write_all(&[0x89, b'P', b'N', b'G', 0, 0, 0, 0]).unwrap();
            w.start_file("ppt/media/image2.jpeg", opts).unwrap();
            w.write_all(&[0xff, 0xd8, 0xff, 0]).unwrap();
            w.finish().unwrap();
        }
        // Write to a temp file because ZipArchive::new wants Seek+Read on
        // an owned source matching the existing function signature.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &zip_bytes).unwrap();
        let f = std::fs::File::open(tmp.path()).unwrap();
        let mut zip = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();
        let map = build_zip_media_buckets(&mut zip, &names, OfficeKind::Pptx);
        assert_eq!(map.get("ppt/media/image1.png").map(String::as_str), Some("slide-001"));
        assert_eq!(map.get("ppt/media/image2.jpeg").map(String::as_str), Some("slide-002"));
    }

    #[test]
    fn builds_xlsx_media_buckets_via_drawing_hop() {
        use std::io::Write;
        let mut zip_bytes: Vec<u8> = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut w = zip::ZipWriter::new(cursor);
            let opts: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            w.start_file("xl/worksheets/_rels/sheet1.xml.rels", opts).unwrap();
            w.write_all(
                br#"<?xml version="1.0"?><Relationships>
                    <Relationship Id="r1" Target="../drawings/drawing1.xml"/>
                </Relationships>"#,
            )
            .unwrap();
            w.start_file("xl/drawings/_rels/drawing1.xml.rels", opts).unwrap();
            w.write_all(
                br#"<?xml version="1.0"?><Relationships>
                    <Relationship Id="r1" Target="../media/image5.png"/>
                </Relationships>"#,
            )
            .unwrap();
            w.start_file("xl/drawings/drawing1.xml", opts).unwrap();
            w.write_all(b"<xml/>").unwrap();
            w.start_file("xl/media/image5.png", opts).unwrap();
            w.write_all(&[0x89, b'P', b'N', b'G', 0]).unwrap();
            w.finish().unwrap();
        }
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &zip_bytes).unwrap();
        let f = std::fs::File::open(tmp.path()).unwrap();
        let mut zip = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();
        let map = build_zip_media_buckets(&mut zip, &names, OfficeKind::Xlsx);
        assert_eq!(map.get("xl/media/image5.png").map(String::as_str), Some("sheet-001"));
    }
}
