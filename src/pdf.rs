use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use lopdf::{Dictionary, Document as PdfDoc, Object, ObjectId};

use crate::error::{ParseError, Result};

pub fn extract_to_dir(input: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();

    let mut pdf = PdfDoc::load(input).map_err(|e| ParseError::Pdf(e.to_string()))?;
    if pdf.is_encrypted() {
        pdf.decrypt("").map_err(|e| ParseError::Pdf(e.to_string()))?;
    }

    let pages = extract_pages_safe(&pdf)?;
    let pages = if pages.is_empty() { vec![String::new()] } else { pages };

    let images_dir = dest.join("images");
    let extracted_images = extract_pdf_images(&pdf, &images_dir).unwrap_or_default();
    let embeds_dir = dest.join("embeddings");
    let extracted_embeds = extract_pdf_embedded_files(&pdf, &embeds_dir).unwrap_or_default();

    for (i, page) in pages.iter().enumerate() {
        let path = dest.join(format!("page-{:03}.md", i + 1));
        let mut md = String::new();
        md.push_str(&format!("# Page {}\n\n", i + 1));
        for line in page.lines() {
            md.push_str(line.trim_end());
            md.push('\n');
        }
        fs::write(&path, md)?;
        written.push(path);
    }

    if !extracted_images.is_empty() {
        let path = dest.join("images.md");
        let mut md = String::from("# Extracted images\n\n");
        for rel in &extracted_images {
            md.push_str(&format!("![]({rel})\n\n"));
        }
        fs::write(&path, md)?;
        written.push(path);
    }

    if !extracted_embeds.is_empty() {
        let path = dest.join("embeddings.md");
        let mut md = String::from("# Embedded files\n\n");
        for (rel, original) in &extracted_embeds {
            md.push_str(&format!("- [{original}]({rel})\n"));
        }
        fs::write(&path, md)?;
        written.push(path);
    }

    Ok(written)
}

fn extract_pdf_embedded_files(pdf: &PdfDoc, dest: &Path) -> Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut counter: u32 = 0;

    // Walk every Filespec dict / EmbeddedFile stream in the document.
    // PDF spec: file specs live under /Catalog/Names/EmbeddedFiles or as
    // /Annot /Subtype /FileAttachment. Easier: scan every dictionary for /EF.
    for (_id, obj) in pdf.objects.iter() {
        let Object::Dictionary(dict) = obj else {
            continue;
        };
        let Ok(ef) = dict.get(b"EF") else { continue };
        let Ok(ef_dict) = ef.as_dict() else { continue };
        let stream_ref = ef_dict.get(b"UF").or_else(|_| ef_dict.get(b"F")).ok();
        let Some(stream_obj) = stream_ref else {
            continue;
        };
        let Ok(stream_id) = stream_obj.as_reference() else {
            continue;
        };
        let Ok(Object::Stream(stream)) = pdf.get_object(stream_id) else {
            continue;
        };
        let bytes = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        if bytes.is_empty() {
            continue;
        }
        let original = dict
            .get(b"UF")
            .or_else(|_| dict.get(b"F"))
            .ok()
            .and_then(|o| o.as_str().ok())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| format!("file-{counter:03}.bin"));
        if !dest.exists() {
            fs::create_dir_all(dest)?;
        }
        counter += 1;
        let safe = crate::util::sanitize_filename(&original);
        let filename = format!("embed-{counter:03}-{safe}");
        fs::write(dest.join(&filename), &bytes)?;
        out.push((format!("embeddings/{filename}"), original));
    }
    Ok(out)
}

fn extract_pages_safe(pdf: &PdfDoc) -> Result<Vec<String>> {
    let extract_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let page_map = pdf.get_pages();
        let mut pages = Vec::with_capacity(page_map.len());
        for page_num in page_map.keys() {
            let mut page = String::new();
            {
                let mut output = pdf_extract::PlainTextOutput::new(&mut page);
                // A single malformed page must not abort the whole document
                // (mirrors pdf_extract::extract_text_by_pages' lenient walk):
                // keep whatever text was captured before the error and move on.
                let _ = pdf_extract::output_doc_page(pdf, &mut output, *page_num);
            }
            pages.push(page);
        }
        pages
    }));
    match extract_result {
        Ok(pages) => Ok(pages),
        Err(panic) => {
            let msg = panic_message(&panic);
            Err(ParseError::Pdf(format!(
                "PDF text extraction failed: {msg}. \
                 Note: pure-Rust PDF extraction has limited CJK CID-font support; \
                 ensure the PDF embeds a ToUnicode CMap."
            )))
        }
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_string()
    }
}

fn extract_pdf_images(pdf: &PdfDoc, images_dir: &Path) -> Result<Vec<String>> {
    let image_to_page = build_image_page_map(pdf);

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut counter: u32 = 0;

    for (&id, obj) in pdf.objects.iter() {
        if seen.contains(&id) {
            continue;
        }
        let Object::Stream(stream) = obj else { continue };
        let is_image = stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| n == b"Image")
            .unwrap_or(false);
        if !is_image {
            continue;
        }

        let filter_name = stream.dict.get(b"Filter").ok().and_then(filter_to_name);

        let (ext, bytes_opt): (&str, Option<Vec<u8>>) = match filter_name.as_deref() {
            Some("DCTDecode") => ("jpg", Some(stream.content.clone())),
            Some("JPXDecode") => ("jp2", Some(stream.content.clone())),
            Some("CCITTFaxDecode") => ("tif", None),
            _ => {
                let decoded = stream.decompressed_content().ok();
                if let Some(decoded) = decoded {
                    match try_encode_raster_as_png(&stream.dict, &decoded) {
                        Some(png) => ("png", Some(png)),
                        None => ("bin", None),
                    }
                } else {
                    ("bin", None)
                }
            }
        };
        let Some(bytes) = bytes_opt else {
            seen.insert(id);
            continue;
        };
        if bytes.is_empty() {
            seen.insert(id);
            continue;
        }
        let bucket = match image_to_page.get(&id) {
            Some(page_num) => format!("page-{:03}", page_num),
            None => "page-unknown".to_string(),
        };
        let bucket_dir = images_dir.join(&bucket);
        if !bucket_dir.exists() {
            fs::create_dir_all(&bucket_dir)?;
        }
        counter += 1;
        let filename = format!("img-{:03}.{ext}", counter);
        fs::write(bucket_dir.join(&filename), &bytes)?;
        out.push(format!("images/{bucket}/{filename}"));
        seen.insert(id);
    }
    Ok(out)
}

/// Walk every page's Resources / XObject dictionary and record the first page
/// number each image-stream `ObjectId` appears on. Form-XObjects are recursed
/// into so images nested inside form groups still get attributed correctly.
fn build_image_page_map(pdf: &PdfDoc) -> HashMap<ObjectId, u32> {
    let mut map: HashMap<ObjectId, u32> = HashMap::new();
    for (&page_num, &page_id) in pdf.get_pages().iter() {
        let Ok((own, inherited)) = pdf.get_page_resources(page_id) else {
            continue;
        };
        let mut visited: HashSet<ObjectId> = HashSet::new();
        if let Some(d) = own {
            collect_xobjects(pdf, d, page_num, &mut map, &mut visited);
        }
        for id in inherited {
            if let Ok(Object::Dictionary(d)) = pdf.get_object(id) {
                collect_xobjects(pdf, d, page_num, &mut map, &mut visited);
            }
        }
    }
    map
}

fn collect_xobjects(
    pdf: &PdfDoc,
    dict: &Dictionary,
    page_num: u32,
    map: &mut HashMap<ObjectId, u32>,
    visited: &mut HashSet<ObjectId>,
) {
    let Ok(xobj) = dict.get(b"XObject") else { return };
    let xobj_dict: &Dictionary = match xobj {
        Object::Dictionary(d) => d,
        Object::Reference(id) => match pdf.get_object(*id) {
            Ok(Object::Dictionary(d)) => d,
            _ => return,
        },
        _ => return,
    };
    for (_, val) in xobj_dict.iter() {
        let Ok(ref_id) = val.as_reference() else { continue };
        if !visited.insert(ref_id) {
            continue;
        }
        map.entry(ref_id)
            .and_modify(|p| {
                if page_num < *p {
                    *p = page_num;
                }
            })
            .or_insert(page_num);
        // Recurse into Form XObjects so nested images get the right page.
        if let Ok(Object::Stream(stream)) = pdf.get_object(ref_id) {
            let is_form = stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| n == b"Form")
                .unwrap_or(false);
            if is_form && let Ok(res) = stream.dict.get(b"Resources") {
                let res_dict: Option<&Dictionary> = match res {
                    Object::Dictionary(d) => Some(d),
                    Object::Reference(id) => match pdf.get_object(*id) {
                        Ok(Object::Dictionary(d)) => Some(d),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(d) = res_dict {
                    collect_xobjects(pdf, d, page_num, map, visited);
                }
            }
        }
    }
}

fn filter_to_name(obj: &Object) -> Option<String> {
    if let Ok(name) = obj.as_name() {
        return Some(String::from_utf8_lossy(name).into_owned());
    }
    if let Ok(arr) = obj.as_array()
        && let Some(last) = arr.last()
        && let Ok(name) = last.as_name()
    {
        return Some(String::from_utf8_lossy(name).into_owned());
    }
    None
}

fn try_encode_raster_as_png(dict: &lopdf::Dictionary, raw: &[u8]) -> Option<Vec<u8>> {
    let w = dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let h = dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    let bpc = dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(8);
    if bpc != 8 {
        return None;
    }
    let cs_name = dict.get(b"ColorSpace").ok().and_then(filter_to_name);
    let (components, color): (usize, image::ColorType) = match cs_name.as_deref() {
        Some("DeviceGray") | Some("CalGray") => (1, image::ColorType::L8),
        Some("DeviceRGB") | Some("CalRGB") => (3, image::ColorType::Rgb8),
        Some("DeviceCMYK") => return None,
        _ => return None,
    };
    let expected = (w as usize) * (h as usize) * components;
    if raw.len() < expected {
        return None;
    }
    let mut buf: Vec<u8> = Vec::new();
    {
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        use image::ImageEncoder;
        encoder.write_image(&raw[..expected], w, h, color.into()).ok()?;
    }
    Some(buf)
}
