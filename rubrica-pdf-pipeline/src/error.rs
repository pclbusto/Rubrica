use thiserror::Error;

#[derive(Error, Debug)]
pub enum PdfPipelineError {
    #[error("Fallo en extracción de PDF: {0}")]
    ExtractionFailed(String),

    #[error("OCR no disponible: {0}")]
    OcrUnavailable(String),

    #[error("Inferencia de LLM fallida: {0}")]
    LlmInferenceFailed(String),

    #[error("Ensamblado de EPUB fallido: {0}")]
    AssemblyFailed(String),

    #[error("Validación XML fallida para chunk {chunk_index}: {reason}")]
    XmlValidationFailed { chunk_index: usize, reason: String },

    #[error("Modelo no encontrado en {path}")]
    ModelNotFound { path: std::path::PathBuf },

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Path error: {0}")]
    Path(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[cfg(feature = "candle-llm")]
    #[error(transparent)]
    Candle(#[from] candle_core::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
