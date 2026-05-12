use std::path::PathBuf;
use tokimo_package_fileparser::parse;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn run(name: &str) -> (PathBuf, Vec<PathBuf>) {
    let out_dir = tempfile::tempdir().expect("tempdir");
    let input = fixtures().join(name);
    let result = parse(&input, out_dir.path()).expect("parse");
    let leaked = out_dir.keep();
    let folder = leaked.join(result.dir.file_name().unwrap());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(|r| r.ok().map(|e| e.path()))
        .collect();
    files.sort();
    (folder, files)
}

fn read_all_md(folder: &PathBuf) -> String {
    let mut buf = String::new();
    let mut entries: Vec<_> = std::fs::read_dir(folder).unwrap().filter_map(|r| r.ok()).collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("md") {
            buf.push_str(&std::fs::read_to_string(&p).unwrap());
            buf.push('\n');
        }
    }
    buf
}

#[test]
fn parses_txt() {
    let (folder, _) = run("sample.txt");
    let md = read_all_md(&folder);
    assert!(md.contains("中文"), "missing chinese: {md}");
}

#[test]
fn parses_markdown_passthrough() {
    let (folder, _) = run("sample.md");
    let md = read_all_md(&folder);
    assert!(md.contains("# 中文标题"));
}

#[test]
fn parses_csv() {
    let (folder, _) = run("sample.csv");
    let md = read_all_md(&folder);
    assert!(md.contains("| 姓名 |"), "csv md: {md}");
    assert!(md.contains("张三"));
}

#[cfg(feature = "html")]
#[test]
fn parses_html() {
    let (folder, _) = run("sample.html");
    let md = read_all_md(&folder);
    assert!(md.contains("中文 HTML 标题"));
    assert!(!md.contains("console.log"), "script must be stripped: {md}");
}

#[cfg(not(feature = "html"))]
#[test]
fn html_is_rejected_without_feature() {
    let tmp = tempfile::tempdir().unwrap();
    let err = parse(fixtures().join("sample.html"), tmp.path()).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("html"), "expected html-feature message, got: {msg}");
}

#[test]
fn parses_json() {
    let (folder, _) = run("sample.json");
    let md = read_all_md(&folder);
    assert!(md.contains("张三"));
    assert!(md.contains("```json"));
}

#[test]
fn parses_docx_with_image() {
    let (folder, _) = run("sample.docx");
    let md = read_all_md(&folder);
    assert!(md.contains("中文 Word 测试文档"), "docx md: {md}");
    assert!(md.contains("张三"));
    let images = folder.join("images");
    assert!(images.exists(), "expected images dir for docx");
    let imgs: Vec<_> = std::fs::read_dir(&images).unwrap().filter_map(|r| r.ok()).collect();
    assert!(!imgs.is_empty(), "expected at least one image extracted");
}

#[test]
fn parses_xlsx_multiple_sheets() {
    let (folder, files) = run("sample.xlsx");
    let mds: Vec<_> = files
        .iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    assert!(mds.len() >= 2, "expected one md per sheet, got: {:?}", mds);
    let md = read_all_md(&folder);
    assert!(md.contains("姓名"));
    assert!(md.contains("北京"));
}

#[test]
fn parses_pptx_multiple_slides() {
    let (folder, files) = run("sample.pptx");
    let mds: Vec<_> = files
        .iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    assert!(mds.len() >= 2, "expected one md per slide, got: {:?}", mds);
    let md = read_all_md(&folder);
    assert!(md.contains("中文 PPT 测试"));
    assert!(md.contains("议程"));
}

#[test]
fn parses_pdf_multiple_pages() {
    let (folder, files) = run("sample.pdf");
    let mds: Vec<_> = files
        .iter()
        .filter(|p| {
            p.extension().and_then(|s| s.to_str()) == Some("md")
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .starts_with("page-")
        })
        .collect();
    assert!(mds.len() >= 2, "expected per-page md files, got: {:?}", mds);
    let md = read_all_md(&folder);
    assert!(md.contains("中文 PDF 测试文档"));
    assert!(md.contains("Hello, world!"));
    assert!(md.contains("Rust 是一门系统编程语言"));
}

#[test]
fn splits_long_docx_into_pages() {
    let (folder, files) = run("big.docx");
    let pages: Vec<_> = files
        .iter()
        .filter(|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            (name.starts_with("page-") || name.starts_with("chapter-")) && name.ends_with(".md")
        })
        .collect();
    assert!(
        pages.len() >= 5,
        "expected >=5 page/chapter files for big.docx, got: {:?}",
        pages
    );
    let md = read_all_md(&folder);
    assert!(md.contains("Page 1"));
    assert!(md.contains("Page 5"));
}

#[test]
fn parse_output_has_json_and_tree() {
    let out_dir = tempfile::tempdir().unwrap();
    let res = parse(fixtures().join("sample.pdf"), out_dir.path()).unwrap();
    assert!(res.dir.exists());
    assert!(res.tree.contains("page-001.md"));
    let json = res.to_json();
    assert!(json.contains("\"dir\""));
    assert!(json.contains("\"files\""));
    assert!(json.contains("\"tree\""));
    assert!(json.contains("page-001.md"));
}

#[test]
fn rejects_unknown_extension() {
    let tmp = tempfile::NamedTempFile::with_suffix(".xyz").unwrap();
    let out = tempfile::tempdir().unwrap();
    let err = parse(tmp.path(), out.path()).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("unsupported"), "msg: {msg}");
}
