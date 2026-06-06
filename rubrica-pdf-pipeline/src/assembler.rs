use crate::{PdfPipelineError, PipelineStatus};
use anyhow::Result;
use gutencore::GutenCore;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::Sender;
use tracing::{error, info, warn};

/// Ensamblador: valida XHTML, lo inyecta en GutenCore y empaqueta el EPUB final.
pub struct Assembler;

impl Assembler {
    /// Toma los fragmentos XHTML (uno por chunk), los valida, los inyecta en un proyecto
    /// GutenCore temporal, y devuelve la ruta al EPUB final + su hash SHA-256.
    pub async fn assemble(
        fragments: Vec<String>,
        title: &str,
        lang: &str,
        output_dir: &Path,
        tx: Sender<PipelineStatus>,
    ) -> Result<(PathBuf, String), PdfPipelineError> {
        let _ = tx.send(PipelineStatus::AssemblyStarted).await;

        // 1. Crear proyecto temporal GutenCore.
        let temp_dir = tempfile::tempdir()?;
        let project_path = temp_dir.path().join("book");

        let mut core = tokio::task::spawn_blocking({
            let title = title.to_string();
            let lang = lang.to_string();
            let project_path = project_path.clone();
            move || GutenCore::new_project(&project_path, &title, &lang)
        })
        .await
        .map_err(|e| PdfPipelineError::AssemblyFailed(e.to_string()))?
        .map_err(|e| PdfPipelineError::AssemblyFailed(format!("GutenCore::new_project: {}", e)))?;

        // 2. Validar e inyectar cada fragmento.
        let mut valid_ids = Vec::new();
        for (idx, fragment) in fragments.into_iter().enumerate() {
            let doc_id = format!("chunk_{:03}", idx);

            match Self::validate_and_repair(&fragment, idx) {
                Ok(repaired) => {
                    let _ = tx
                        .send(PipelineStatus::XmlValidated { chunk_index: idx })
                        .await;

                    // Inyectar en GutenCore (bloqueante, va dentro de spawn_blocking si es pesado).
                    core.add_document(&doc_id, &repaired)
                        .map_err(|e| {
                            PdfPipelineError::AssemblyFailed(format!(
                                "add_document({}) error: {}",
                                doc_id, e
                            ))
                        })?;
                    valid_ids.push(doc_id);
                }
                Err(repair_err) => {
                    warn!(
                        "Chunk {} no pasó validación XML: {}. Aplicando heurística de emergencia...",
                        idx, repair_err
                    );
                    let emergency = Self::emergency_heuristic_cleanup(&fragment);
                    // Intentar una última vez con la salida de emergencia.
                    if Self::validate_and_repair(&emergency, idx).is_ok() {
                        let _ = tx
                            .send(PipelineStatus::XmlHeuristicRecovered { chunk_index: idx })
                            .await;
                        core.add_document(&doc_id, &emergency)
                            .map_err(|e| {
                                PdfPipelineError::AssemblyFailed(format!(
                                    "add_document({}) tras heurística: {}",
                                    doc_id, e
                                ))
                            })?;
                        valid_ids.push(doc_id);
                    } else {
                        error!("Chunk {} irreparable. Saltando.", idx);
                    }
                }
            }
        }

        if valid_ids.is_empty() {
            return Err(PdfPipelineError::AssemblyFailed(
                "Ningún fragmento pasó la validación XML".into(),
            ));
        }

        // 3. Reconstruir spine para que incluya solo nuestros chunks en orden.
        core.set_spine(valid_ids);

        // 4. Regenerar nav.xhtml a partir de los h1/h2 reales de nuestros chunks.
        //    GutenCore::save() NO llama update_nav() automáticamente.
        core.update_nav()
            .map_err(|e| PdfPipelineError::AssemblyFailed(format!("update_nav: {}", e)))?;

        // 5. Guardar proyecto descomprimido.
        core.save()
            .map_err(|e| PdfPipelineError::AssemblyFailed(format!("save error: {}", e)))?;

        // 5. Empaquetar en EPUB (ZIP con mimetype sin compresión al inicio).
        let epub_path = output_dir.join(format!("{}.epub", sanitize_filename::sanitize(title)));
        let epub_path_clone = epub_path.clone();
        let project_path_clone = project_path.clone();

        tokio::task::spawn_blocking(move || {
            Self::package_epub(&project_path_clone, &epub_path_clone)
        })
        .await
        .map_err(|e| PdfPipelineError::AssemblyFailed(format!("zip task: {}", e)))??;

        // 6. Calcular SHA-256 del EPUB final.
        let hash = Self::compute_sha256(&epub_path)?;
        info!("EPUB ensamblado en {:?} (SHA-256: {})", epub_path, hash);

        Ok((epub_path, hash))
    }

    /// Valida que un fragmento sea XHTML bien formado usando roxmltree.
    /// Si el fragmento parece Markdown (no empieza con una etiqueta XML), lo convierte primero.
    /// Antes de parsear, limpia bloques de código markdown (```xml) que el LLM suele alucinar.
    /// Si está roto, intenta repararlo con una envoltura de emergencia.
    fn validate_and_repair(fragment: &str, chunk_index: usize) -> Result<String, String> {
        let stripped = Self::strip_markdown_fences(fragment);
        let content = if Self::looks_like_markdown(&stripped) {
            markdown_to_xhtml(&stripped)
        } else {
            stripped
        };
        // Envolvemos en un div para que roxmltree pueda parsear fragmentos sueltos.
        let wrapped = format!("<div xmlns=\"http://www.w3.org/1999/xhtml\">{}</div>", content);
        match roxmltree::Document::parse(&wrapped) {
            Ok(_) => Ok(content),
            Err(e) => Err(format!("Chunk {} XML parse error: {}", chunk_index, e)),
        }
    }

    /// Devuelve true si el fragmento parece Markdown en lugar de XHTML.
    fn looks_like_markdown(s: &str) -> bool {
        s.lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .map(|first| !first.starts_with('<'))
            .unwrap_or(false)
    }

    /// Elimina envoltorios de bloques de código markdown que los LLMs suelen generar.
    fn strip_markdown_fences(fragment: &str) -> String {
        // ``` con lenguaje opcional (xml, xhtml, html, markdown, vacío)
        let re = regex::Regex::new(r"(?s)```[a-z]*\n?(.*?)\n?```").unwrap();
        let stripped = if let Some(caps) = re.captures(fragment) {
            caps.get(1).map(|m| m.as_str().trim()).unwrap_or(fragment.trim())
        } else {
            fragment.trim()
        };
        // Eliminar separadores --- sueltos que Ollama genera
        let re_sep = regex::Regex::new(r"(?m)^\s*---+\s*$").unwrap();
        re_sep.replace_all(stripped, "").trim().to_string()
    }

    /// Limpieza heurística de emergencia cuando el LLM alucinó tags rotos.
    fn emergency_heuristic_cleanup(fragment: &str) -> String {
        let mut text = fragment.to_string();

        // Cerrar tags comunes abiertos (p, h1, h2, h3, div, span, strong, em).
        let tags = ["p", "h1", "h2", "h3", "div", "span", "strong", "em"];
        for tag in tags {
            let open_count = text.matches(&format!("<{}>", tag)).count()
                + text.matches(&format!("<{} ", tag)).count();
            let close_count = text.matches(&format!("</{}>", tag)).count();
            let missing = open_count.saturating_sub(close_count);
            for _ in 0..missing {
                text.push_str(&format!("</{}>\n", tag));
            }
        }

        // Eliminar tags desconocidos o potencialmente peligrosos (script, iframe, object).
        let re_dangerous = regex::Regex::new(r"<script[^>]*>.*?</script>|<iframe[^>]*>.*?</iframe>|<object[^>]*>.*?</object>").unwrap();
        text = re_dangerous.replace_all(&text, "").to_string();

        // Eliminar atributos de evento (onclick, onload, etc.).
        let re_events = regex::Regex::new(r#"\s+on\w+=["'][^"']*["']"#).unwrap();
        text = re_events.replace_all(&text, "").to_string();

        text
    }

    /// Empaqueta una carpeta de proyecto EPUB en un archivo .epub (ZIP).
    fn package_epub(source_dir: &Path, epub_path: &Path) -> Result<(), PdfPipelineError> {
        let file = fs::File::create(epub_path)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // mimetype va sin compresión y primero.
        let mimetype_path = source_dir.join("mimetype");
        if mimetype_path.exists() {
            zip.start_file(
                "mimetype",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )?;
            zip.write_all(&fs::read(&mimetype_path)?)?;
        }

        for entry in walkdir::WalkDir::new(source_dir) {
            let entry = entry.map_err(|e| PdfPipelineError::Path(e.to_string()))?;
            let path = entry.path();
            if path.is_file() && path.file_name() != Some(std::ffi::OsStr::new("mimetype")) {
                let relative = path
                    .strip_prefix(source_dir)
                    .map_err(|e| PdfPipelineError::Path(e.to_string()))?;
                let rel_str = relative.to_string_lossy();
                // Excluir archivos temporales de SQLite de GutenCore
                if rel_str.starts_with(".gutenair.db") {
                    continue;
                }
                zip.start_file(
                    rel_str.replace("\\", "/"),
                    options,
                )?;
                zip.write_all(&fs::read(path)?)?;
            }
        }

        zip.finish()?;
        Ok(())
    }

    fn compute_sha256(path: &Path) -> Result<String, PdfPipelineError> {
        let bytes = fs::read(path)?;
        let hash = Sha256::digest(&bytes);
        Ok(hex::encode(hash))
    }
}

/// Convierte Markdown estándar a fragmentos XHTML válidos para EPUB.
/// Maneja: encabezados (#/##/###), párrafos, negrita (**), cursiva (*).
pub fn markdown_to_xhtml(md: &str) -> String {
    let mut out = String::new();

    for block in md.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let first_line = block.lines().next().unwrap_or("").trim();

        if first_line.starts_with("### ") {
            let title = xml_escape(first_line.trim_start_matches("### ").trim());
            out.push_str(&format!("<h3>{}</h3>\n", title));
            append_remainder(&mut out, block);
        } else if first_line.starts_with("## ") {
            let title = xml_escape(first_line.trim_start_matches("## ").trim());
            out.push_str(&format!("<h2>{}</h2>\n", title));
            append_remainder(&mut out, block);
        } else if first_line.starts_with("# ") {
            let title = xml_escape(first_line.trim_start_matches("# ").trim());
            out.push_str(&format!("<h1>{}</h1>\n", title));
            append_remainder(&mut out, block);
        } else {
            let text = block
                .lines()
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join(" ");
            let text = apply_inline_md(&text);
            out.push_str(&format!("<p>{}</p>\n", text));
        }
    }

    out
}

fn append_remainder(out: &mut String, block: &str) {
    let rest: String = block.lines().skip(1).map(|l| l.trim()).collect::<Vec<_>>().join(" ");
    if !rest.trim().is_empty() {
        out.push_str(&format!("<p>{}</p>\n", apply_inline_md(rest.trim())));
    }
}

fn apply_inline_md(s: &str) -> String {
    let s = xml_escape(s);
    let re_bold = regex::Regex::new(r"\*\*(.+?)\*\*").unwrap();
    let s = re_bold.replace_all(&s, "<strong>$1</strong>");
    let re_em = regex::Regex::new(r"\*(.+?)\*").unwrap();
    re_em.replace_all(&s, "<em>$1</em>").into_owned()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
