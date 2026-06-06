use crate::{ExtractedPage, ExtractionSource, PipelineConfig, PdfPipelineError};
use anyhow::Result;
use std::path::Path;
use tracing::{debug, info, warn};

#[cfg(feature = "ort-ocr")]
use std::sync::{Arc, Mutex, OnceLock};

/// Cache global de sesiones ONNX OCR por ruta de modelo.
/// Evita recrear el entorno C++ de ONNX Runtime en cada extracción,
/// lo cual causa fugas de memoria y panics por doble inicialización.
#[cfg(feature = "ort-ocr")]
static OCR_SESSION_CACHE: OnceLock<Mutex<std::collections::HashMap<std::path::PathBuf, Arc<ort::Session>>>> = OnceLock::new();

/// Motor de extracción de contenido desde archivos PDF.
pub struct PdfExtractor;

impl PdfExtractor {
    /// Extrae texto de un PDF. Intenta pdftotext (poppler) primero por su robustez;
    /// si no está disponible o falla, cae a pdf-extract.
    pub async fn extract(
        path: &Path,
        config: &PipelineConfig,
    ) -> Result<Vec<ExtractedPage>, PdfPipelineError> {
        info!("Iniciando extracción de {:?}", path);

        let text = {
            let path_buf = path.to_path_buf();
            tokio::task::spawn_blocking(move || {
                Self::extract_text_pdftotext(&path_buf)
                    .unwrap_or_else(|_| Self::extract_text_pdf_extract(&path_buf))
            })
            .await
            .map_err(|e| PdfPipelineError::ExtractionFailed(e.to_string()))?
        };

        let pages = Self::split_into_pages(&text);

        let total_chars: usize = pages.iter().map(|p| p.text.len()).sum();
        let density = if pages.is_empty() {
            0.0
        } else {
            total_chars as f64 / pages.len().max(1) as f64
        };

        debug!("Densidad de texto promedio por página: {:.2}", density);

        if density < config.ocr_fallback_threshold * 1000.0 {
            warn!("Densidad de texto baja ({:.2}). Evaluando OCR fallback...", density);

            #[cfg(feature = "ort-ocr")]
            {
                if let Some(ref model_path) = config.onnx_ocr_model {
                    return Self::extract_with_ort(path, model_path).await;
                } else {
                    warn!("Modelo ONNX OCR no configurado (onnx_ocr_model: None). Devolviendo texto crudo.");
                }
            }
            #[cfg(not(feature = "ort-ocr"))]
            {
                warn!("OCR no compilado (feature 'ort-ocr' desactivada). Devolviendo texto crudo.");
            }
        }

        Ok(pages)
    }

    /// Genera XML de layout usando `pdftohtml -xml` (poppler).
    /// El XML incluye coordenadas, tamaños de fuente y formato de cada elemento de texto.
    ///
    /// pdftohtml puede retornar exit code no-cero si hay páginas con imágenes que no puede
    /// extraer (DRM en imágenes, páginas escaneadas, etc.) pero igualmente genera un XML
    /// válido con el texto. Por eso no se chequea el exit code — solo si el XML existe.
    pub fn extract_xml(path: &Path) -> Result<String, String> {
        let temp_dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let output_base = temp_dir.path().join("doc");

        let output = std::process::Command::new("pdftohtml")
            .args([
                "-i",   // ignorar imágenes (evita fallos por páginas con imágenes embedded)
                "-xml", // salida en formato XML con coordenadas y fonts
                path.to_str().ok_or("path inválido")?,
                output_base.to_str().ok_or("path inválido")?,
            ])
            .output()
            .map_err(|e| format!("pdftohtml no encontrado: {}", e))?;

        let xml_path = output_base.with_extension("xml");

        if xml_path.exists() {
            return std::fs::read_to_string(&xml_path).map_err(|e| e.to_string());
        }

        // El XML no fue generado — reportar por qué
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "pdftohtml (exit={}) no generó XML: {}",
            output.status.code().unwrap_or(-1),
            if stderr.trim().is_empty() { "sin detalle en stderr".into() } else { stderr.trim().to_string() }
        ))
    }

    /// Extrae texto usando pdftotext (poppler). Cada página separada por \x0C.
    fn extract_text_pdftotext(path: &std::path::PathBuf) -> Result<String, String> {
        let output = std::process::Command::new("pdftotext")
            .args(["-nodrm", "-enc", "UTF-8"])
            .arg(path)
            .arg("-") // stdout
            .output()
            .map_err(|e| e.to_string())?;

        if !output.status.success() {
            return Err(format!(
                "pdftotext falló: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let text = String::from_utf8_lossy(&output.stdout).to_string();
        // Validar que haya texto real (no solo whitespace)
        if text.lines().any(|l| !l.trim().is_empty()) {
            Ok(text)
        } else {
            Err("pdftotext no produjo texto legible".into())
        }
    }

    /// Extrae texto usando pdf-extract (fallback).
    fn extract_text_pdf_extract(path: &std::path::PathBuf) -> String {
        pdf_extract::extract_text(path).unwrap_or_default()
    }

    /// Divide el texto plano extraído en páginas aproximadas usando saltos de form-feed
    /// o heurísticas de espaciado. `pdf-extract` no siempre preserva delimitadores de página,
    /// así que este método es un best-effort.
    fn split_into_pages(full_text: &str) -> Vec<ExtractedPage> {
        let raw_pages: Vec<String> = if full_text.contains('\x0C') {
            full_text.split('\x0C').map(|s| s.to_string()).collect()
        } else {
            let chars: Vec<char> = full_text.chars().collect();
            if chars.len() < 500 {
                vec![chars.iter().collect()]
            } else {
                chars
                    .chunks(3000)
                    .map(|chunk| chunk.iter().collect())
                    .collect()
            }
        };

        raw_pages
            .into_iter()
            .enumerate()
            .map(|(i, text)| ExtractedPage {
                page_number: i + 1,
                text,
                source: ExtractionSource::NativeText,
            })
            .collect()
    }

    /// Obtiene o crea una sesión ONNX cacheada para OCR.
    /// El cache evita la recreación costosa del runtime C++ en cada llamada.
    #[cfg(feature = "ort-ocr")]
    fn get_cached_ort_session(model_path: &Path) -> Result<Arc<ort::Session>, PdfPipelineError> {
        let cache = OCR_SESSION_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
        let mut map = cache.lock().map_err(|e| PdfPipelineError::OcrUnavailable(format!("Mutex poison: {}", e)))?;

        if let Some(session) = map.get(model_path) {
            debug!("Reutilizando sesión ONNX cacheada para {:?}", model_path);
            return Ok(Arc::clone(session));
        }

        info!("Creando nueva sesión ONNX OCR desde {:?}", model_path);
        let session = ort::Session::builder()
            .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?
            .commit_from_file(model_path)
            .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?;
        let session = Arc::new(session);
        map.insert(model_path.to_path_buf(), Arc::clone(&session));
        Ok(session)
    }

    /// Extracción vía ONNX Runtime usando un modelo OCR cuantizado.
    /// Requiere la feature `ort-ocr` y un modelo `.onnx` compatible (ej. PaddleOCR optimizado).
    #[cfg(feature = "ort-ocr")]
    async fn extract_with_ort(
        path: &Path,
        model_path: &Path,
    ) -> Result<Vec<ExtractedPage>, PdfPipelineError> {
        use ort::Value;
        use std::process::Command;

        info!("OCR fallback activado con modelo ONNX: {:?}", model_path);

        // 1. Convertir PDF a imágenes temporales (requiere poppler-utils: pdftoppm).
        let temp_dir = tempfile::tempdir()?;
        let output_prefix = temp_dir.path().join("page");

        let status = Command::new("pdftoppm")
            .args(&["-png", path.to_str().unwrap(), output_prefix.to_str().unwrap()])
            .status()?;

        if !status.success() {
            return Err(PdfPipelineError::OcrUnavailable(
                "pdftoppm falló. ¿Tenés poppler-utils instalado?".into(),
            ));
        }

        // 2. Obtener sesión ONNX cacheada (una sola vez por modelo, thread-safe).
        let session = Self::get_cached_ort_session(model_path)?;

        let mut pages = Vec::new();

        for entry in walkdir::WalkDir::new(temp_dir.path()) {
            let entry = entry.map_err(|e| PdfPipelineError::Io(e.into()))?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("png") {
                let img_path = entry.path().to_path_buf();

                let text = tokio::task::spawn_blocking({
                    let session = Arc::clone(&session);
                    move || {
                        // Preprocesamiento: cargar imagen como tensor [1, C, H, W]
                        // NOTA: Este bloque es específico del modelo ONNX usado.
                        // Para un modelo genérico de OCR (PaddleOCR / TrOCR) se debe adaptar
                        // la normalización y las dimensiones de entrada.
                        let img = image::open(&img_path)
                            .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?;
                        let rgb = img.to_rgb8();
                        let (w, h) = rgb.dimensions();
                        let raw = rgb.into_raw();

                        // Placeholder de tensor de entrada (normalización básica).
                        // En un modelo real, consultá las dimensiones esperadas con `session.inputs()`.
                        let input_tensor = Value::from_array(
                            ndarray::Array4::from_shape_fn((1, 3, h as usize, w as usize), |(_, c, y, x)| {
                                let v = raw[(y * w as usize + x) * 3 + c] as f32;
                                (v - 127.5) / 127.5 // Normalización estándar [-1, 1]
                            }),
                        )
                        .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?;

                        let outputs = session
                            .run(ort::inputs![input_tensor].map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?)
                            .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?;

                        // Placeholder: extraer texto del output del modelo.
                        // Esto depende 100% de la arquitectura del modelo ONNX.
                        let output = outputs
                            .get("output")
                            .or_else(|| outputs.first())
                            .ok_or_else(|| PdfPipelineError::OcrUnavailable("Sin outputs del modelo ONNX".into()))?;

                        let view = output
                            .try_extract_tensor::<f32>()
                            .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))?;

                        // Decodificación best-effort: buscar índices de clase con argmax.
                        // En la práctica, un modelo OCR real requiere un decoder CTC o Attention.
                        let indices: Vec<usize> = view
                            .rows()
                            .into_iter()
                            .map(|row| {
                                row.iter()
                                    .enumerate()
                                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                                    .map(|(i, _)| i)
                                    .unwrap_or(0)
                            })
                            .collect();

                        // Mapeo dummy: debería ser el vocabulario del modelo.
                        let decoded: String = indices
                            .iter()
                            .map(|&i| std::char::from_u32(i as u32 + 0x30).unwrap_or('?'))
                            .collect();

                        Ok::<_, PdfPipelineError>(decoded)
                    }
                })
                .await
                .map_err(|e| PdfPipelineError::OcrUnavailable(e.to_string()))??;

                let page_num = entry
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.rsplit_once('-'))
                    .and_then(|(_, n)| n.parse::<usize>().ok())
                    .unwrap_or(0);

                pages.push(ExtractedPage {
                    page_number: page_num,
                    text,
                    source: ExtractionSource::Ocr,
                });
            }
        }

        pages.sort_by_key(|p| p.page_number);
        Ok(pages)
    }
}
