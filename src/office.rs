use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use office_oxide::Document;
use office_oxide::ir::{DocumentIR, Element, Image, ImageFormat, Section};

use crate::error::{ParseError, Result};
use crate::util::sanitize_filename;

pub fn extract_to_dir(input: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let doc = Document::open(input)?;
    let ir = doc.to_ir();
    let images_dir = dest.join("images");
    let mut written: Vec<PathBuf> = Vec::new();
    let mut image_counter: u32 = 0;

    let single_section = ir.sections.len() == 1;

    for (i, section) in ir.sections.iter().enumerate() {
        let mut section_clone = section.clone();
        let saved = save_section_images(&mut section_clone, &images_dir, &mut image_counter)?;

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
    // (it skips drawing/anchor/inline) – pull everything under `*/media/` via zip.
    let zip_images = extract_zip_media(input, &images_dir, &mut image_counter).unwrap_or_default();
    if !zip_images.is_empty() {
        let path = dest.join("images.md");
        let mut md = String::from("# Embedded media\n\n");
        for rel in &zip_images {
            md.push_str(&format!("![]({rel})\n\n"));
        }
        fs::write(&path, md)?;
        written.push(path);
    }

    Ok(written)
}

fn extract_zip_media(input: &Path, images_dir: &Path, counter: &mut u32) -> Result<Vec<String>> {
    let file = fs::File::open(input)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| ParseError::Zip(e.to_string()))?;
    let mut out = Vec::new();
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    for name in names {
        let lower = name.to_ascii_lowercase();
        let in_media = lower.contains("/media/") && !lower.ends_with('/') && is_image_name(&lower);
        if !in_media {
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
        let ext = Path::new(&name)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .or_else(|| sniff_extension(&buf).map(|s| s.to_string()))
            .unwrap_or_else(|| "bin".to_string());
        if !images_dir.exists() {
            fs::create_dir_all(images_dir)?;
        }
        *counter += 1;
        let filename = format!("img-{:03}.{ext}", *counter);
        fs::write(images_dir.join(&filename), &buf)?;
        out.push(format!("images/{filename}"));
    }
    Ok(out)
}

fn is_image_name(lower: &str) -> bool {
    [
        ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".webp", ".emf", ".wmf", ".svg",
    ]
    .iter()
    .any(|e| lower.ends_with(e))
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

fn save_section_images(section: &mut Section, images_dir: &Path, counter: &mut u32) -> Result<Vec<String>> {
    let mut saved: Vec<String> = Vec::new();
    walk_elements(&mut section.elements, &mut |img| {
        if let Some(rel) = persist_image(img, images_dir, counter)? {
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

fn persist_image(img: &Image, images_dir: &Path, counter: &mut u32) -> Result<Option<String>> {
    let Some(bytes) = img.data.as_ref() else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    if !images_dir.exists() {
        fs::create_dir_all(images_dir)?;
    }
    *counter += 1;
    let ext: &str = match &img.format {
        Some(fmt) => fmt.extension(),
        None => sniff_extension(bytes).unwrap_or("bin"),
    };
    let filename = format!("img-{:03}.{ext}", *counter);
    let abs = images_dir.join(&filename);
    fs::write(&abs, bytes)?;
    Ok(Some(format!("images/{filename}")))
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
