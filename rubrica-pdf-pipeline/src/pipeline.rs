use crate::{
    assembler::Assembler,
    extractor::PdfExtractor,
    semantic::SemanticStructurer,
    xml_layout,
    Chapter, PipelineConfig, PipelineStatus, PdfPipelineError,
};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::{self, Sender};
use tracing::{info, instrument, warn};

pub struct PdfPipeline {
    config: PipelineConfig,
}

impl PdfPipeline {
    pub fn new(config: PipelineConfig) -> Self {
        Self { config }
    }

    pub fn validate_models(&self) -> Result<(), PdfPipelineError> {
        #[cfg(feature = "ort-ocr")]
        if let Some(ref onnx) = self.config.onnx_ocr_model {
            if !onnx.exists() {
                return Err(PdfPipelineError::ModelNotFound { path: onnx.clone() });
            }
            info!("Modelo OCR verificado: {:?}", onnx);
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn convert(
        &self,
        pdf_path: &Path,
        output_dir: &Path,
        title: &str,
        lang: &str,
    ) -> Result<(PathBuf, String), PdfPipelineError> {
        self.validate_models()?;

        let (tx, mut rx) = mpsc::channel::<PipelineStatus>(32);

        let logger = tokio::spawn(async move {
            while let Some(status) = rx.recv().await {
                match &status {
                    PipelineStatus::ChunkProcessed { chunk_index, .. } => {
                        info!("[Pipeline] Capítulo {} procesado", chunk_index + 1);
                    }
                    PipelineStatus::Completed { epub_path, .. } => {
                        info!("[Pipeline] Completado: {:?}", epub_path);
                        break;
                    }
                    _ => info!("[Pipeline] {:?}", status),
                }
            }
        });

        let xhtml_fragments = self.extract_and_structure(pdf_path, tx.clone()).await?;

        let (epub_path, sha256) =
            Assembler::assemble(xhtml_fragments, title, lang, output_dir, tx.clone()).await?;

        let _ = tx
            .send(PipelineStatus::Completed {
                epub_path: epub_path.clone(),
                book_id: None,
            })
            .await;

        drop(tx);
        let _ = logger.await;

        Ok((epub_path, sha256))
    }

    pub async fn convert_with_sender(
        &self,
        pdf_path: &Path,
        output_dir: &Path,
        title: &str,
        lang: &str,
        tx: Sender<PipelineStatus>,
    ) -> Result<(PathBuf, String), PdfPipelineError> {
        self.validate_models()?;

        let xhtml_fragments = self.extract_and_structure(pdf_path, tx.clone()).await?;

        let (epub_path, sha256) =
            Assembler::assemble(xhtml_fragments, title, lang, output_dir, tx.clone()).await?;

        let _ = tx
            .send(PipelineStatus::Completed {
                epub_path: epub_path.clone(),
                book_id: None,
            })
            .await;

        Ok((epub_path, sha256))
    }

    /// Extrae y estructura el PDF en fragmentos XHTML.
    ///
    /// Estrategia:
    /// 1. `pdftohtml -i -xml` → análisis de layout (font size, posición) → capítulos sin LLM
    /// 2. Si falla → `pdftotext` → texto plano en un único capítulo
    async fn extract_and_structure(
        &self,
        pdf_path: &Path,
        tx: Sender<PipelineStatus>,
    ) -> Result<Vec<String>, PdfPipelineError> {
        // ── Intento 1: XML layout (pdftohtml) ────────────────────────────────
        let xml_result = {
            let path_buf = pdf_path.to_path_buf();
            match tokio::task::spawn_blocking(move || PdfExtractor::extract_xml(&path_buf)).await {
                Ok(Ok(xml)) => Some(xml),
                Ok(Err(e)) => { eprintln!("[xml] pdftohtml: {}", e); None }
                Err(e)     => { eprintln!("[xml] spawn: {}", e); None }
            }
        };

        if let Some(xml) = xml_result {
            match xml_layout::parse_and_reconstruct(&xml) {
                Ok(chapters) if !chapters.is_empty() => {
                    info!("XML layout: {} capítulos", chapters.len());
                    let _ = tx.send(PipelineStatus::ExtractionStarted { total_pages_hint: None }).await;
                    let _ = tx.send(PipelineStatus::PatternsDetected { patterns_count: chapters.len() }).await;
                    return Ok(chapters_to_xhtml(chapters, tx).await);
                }
                Ok(_)  => eprintln!("[xml] 0 capítulos detectados"),
                Err(e) => eprintln!("[xml] parse: {}", e),
            }
        }

        // ── Intento 2: pdftotext → texto plano sin estructura de capítulos ───
        warn!("Usando pdftotext como fallback (sin detección de capítulos)");
        let pages = PdfExtractor::extract(pdf_path, &self.config).await?;
        let _ = tx.send(PipelineStatus::ExtractionStarted {
            total_pages_hint: Some(pages.len()),
        }).await;

        let full_text: String = pages.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n");
        if full_text.trim().is_empty() {
            return Err(PdfPipelineError::ExtractionFailed(
                "No se pudo extraer texto del PDF. \
                 El archivo puede ser un PDF escaneado que requiere OCR.".into(),
            ));
        }

        let chapter = Chapter { number: None, title: String::new(), body: full_text };
        let xhtml = SemanticStructurer::structure_chapter(&chapter);
        let _ = tx.send(PipelineStatus::ChunkProcessed { chunk_index: 0, output_chars: xhtml.len() }).await;
        Ok(vec![xhtml])
    }
}

async fn chapters_to_xhtml(chapters: Vec<Chapter>, tx: Sender<PipelineStatus>) -> Vec<String> {
    let mut fragments = Vec::with_capacity(chapters.len());
    for (i, chapter) in chapters.iter().enumerate() {
        let xhtml = SemanticStructurer::structure_chapter(chapter);
        let _ = tx.send(PipelineStatus::ChunkProcessed {
            chunk_index: i,
            output_chars: xhtml.len(),
        }).await;
        fragments.push(xhtml);
    }
    fragments
}
