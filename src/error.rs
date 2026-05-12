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
}

pub type Result<T> = std::result::Result<T, ParseError>;
