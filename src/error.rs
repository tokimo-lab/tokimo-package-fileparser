use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported file extension: {0:?}")]
    UnsupportedExtension(String),

    #[error("missing file extension for {0}")]
    MissingExtension(String),

    #[error("office parse error: {0}")]
    Office(#[from] office_oxide::OfficeError),

    #[error("zip error: {0}")]
    Zip(String),

    #[error("pdf parse error: {0}")]
    Pdf(String),

    #[error("csv parse error: {0}")]
    Csv(#[from] csv::Error),

    #[error("encoding error: {0}")]
    Encoding(String),

    #[error("archive error: {0}")]
    Archive(String),

    #[error(
        "archive too large: {size} bytes exceeds limit of {limit} bytes \
         ({size_human} > {limit_human})",
        size_human = format_bytes(*size),
        limit_human = format_bytes(*limit),
    )]
    ArchiveTooLarge { size: u64, limit: u64 },
}

fn format_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if n >= GIB {
        format!("{:.2} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.2} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.2} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

pub type Result<T> = std::result::Result<T, ParseError>;
