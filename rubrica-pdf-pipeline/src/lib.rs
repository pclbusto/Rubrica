use std::path::PathBuf;

pub mod error;
pub mod extractor;
pub mod semantic;
pub mod xml_layout;
pub mod assembler;
pub mod pipeline;

pub use pipeline::PdfPipeline;
pub use error::PdfPipelineError;

#[derive(Debug, Clone)]
pub struct ExtractedPage {
    pub page_number: usize,
    pub text: String,
    pub source: ExtractionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionSource {
    NativeText,
    Ocr,
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub number: Option<u32>,
    pub title: String,
    pub body: String,
}

/// Configuración del pipeline PDF → EPUB.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Umbral de densidad de texto para decidir si usar OCR.
    pub ocr_fallback_threshold: f64,
    /// Ruta al modelo ONNX para OCR (requiere feature `ort-ocr`).
    pub onnx_ocr_model: Option<PathBuf>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            ocr_fallback_threshold: 0.05,
            onnx_ocr_model: Some(PathBuf::from("/home/pedro/models/ocr/ch_PP-OCRv3_rec_infer.onnx")),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PipelineStatus {
    ExtractionStarted { total_pages_hint: Option<usize> },
    /// Cantidad de capítulos detectados por el analizador de layout.
    PatternsDetected { patterns_count: usize },
    ChunkProcessed { chunk_index: usize, output_chars: usize },
    AssemblyStarted,
    XmlValidated { chunk_index: usize },
    XmlHeuristicRecovered { chunk_index: usize },
    Completed { epub_path: PathBuf, book_id: Option<i64> },
}
