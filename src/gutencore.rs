use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::read::ZipArchive;

/// Metadatos extraídos de un EPUB, combinando Dublin Core y extensiones Rúbrica
#[derive(Debug, Clone)]
pub struct EpubMetadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub identifier: String,
    pub series: Option<String>,      // rubrica:series
    pub progress: Option<String>,    // rubrica:progress
}

/// Adaptador que permite usar GutenCore con archivos `.epub` comprimidos (ZIP).
/// GutenCore nativamente trabaja con directorios descomprimidos; esta capa
/// se encarga de la extracción temporal y del parseo adicional de metadatos.
pub struct GutenAdapter;

impl GutenAdapter {
    /// Abre un archivo `.epub` descomprimiéndolo temporalmente y cargándolo
    /// con GutenCore. Ideal para extracción de metadatos y validación estructural.
    ///
    /// El directorio temporal se limpia automáticamente al salir del scope.
    pub async fn open_epub(epub_path: &str) -> Result<(tempfile::TempDir, gutencore::GutenCore)> {
        let path = PathBuf::from(epub_path);
        tokio::task::spawn_blocking(move || {
            let temp_dir = tempfile::tempdir()?;
            Self::extract_zip(&path, temp_dir.path())?;
            let core = gutencore::GutenCore::open_folder(temp_dir.path())
                .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;
            Ok((temp_dir, core))
        })
        .await?
    }

    /// Valida un EPUB extrayendo todos los metadatos relevantes para Rúbrica.
    /// Combina la carga de GutenCore con un parseo directo del OPF para campos
    /// que GutenCore aún no expone (como dc:creator o metadatos rubrica:*).
    pub async fn validate_epub(epub_path: &str) -> Result<EpubMetadata> {
        let path = PathBuf::from(epub_path);
        tokio::task::spawn_blocking(move || {
            let temp_dir = tempfile::tempdir()?;
            Self::extract_zip(&path, temp_dir.path())?;

            // Cargar con GutenCore para validar estructura
            let core = gutencore::GutenCore::open_folder(temp_dir.path())
                .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;

            let opf_path = core.opf_path.as_ref()
                .context("OPF path not found after loading")?;
            let opf_content = std::fs::read_to_string(opf_path)?;

            // Parseo directo del OPF para autor y extensiones
            let doc = roxmltree::Document::parse(&opf_content)
                .map_err(|e| anyhow::anyhow!("XML parse error: {}", e))?;

            let ns_dc = "http://purl.org/dc/elements/1.1/";
            let ns_opf = "http://www.idpf.org/2007/opf";

            // Extraer dc:creator (autor)
            let author = doc.descendants()
                .find(|n| n.has_tag_name((ns_dc, "creator")))
                .and_then(|n| n.text())
                .unwrap_or("Unknown")
                .trim()
                .to_string();

            // Extraer metadatos rubrica:*
            let mut series = None;
            let mut progress = None;

            for meta in doc.descendants().filter(|n| n.has_tag_name((ns_opf, "meta"))) {
                if let Some(prop) = meta.attribute("property") {
                    if let Some(text) = meta.text() {
                        match prop {
                            "rubrica:series" => series = Some(text.trim().to_string()),
                            "rubrica:progress" => progress = Some(text.trim().to_string()),
                            _ => {}
                        }
                    }
                }
            }

            let metadata = core.get_metadata()
                .context("No metadata loaded by GutenCore")?;

            Ok(EpubMetadata {
                title: metadata.title.clone(),
                author,
                language: metadata.language.clone(),
                identifier: metadata.identifier.clone(),
                series,
                progress,
            })
        })
        .await?
    }

    /// Test de lectura rápido: descomprime el EPUB y verifica que GutenCore
    /// pueda cargarlo sin errores. Útil para validar integridad post-copia.
    pub async fn open_folder(epub_path: &str) -> Result<()> {
        let _ = Self::open_epub(epub_path).await?;
        Ok(())
    }

    /// Extrae un archivo ZIP (EPUB) a un directorio destino.
    fn extract_zip(src: &Path, dest: &Path) -> Result<()> {
        let file = std::fs::File::open(src)?;
        let mut archive = ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let mut zip_file = archive.by_index(i)?;
            let out_path = dest.join(zip_file.name());

            if zip_file.name().ends_with('/') {
                std::fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out_file = std::fs::File::create(&out_path)?;
                let mut buf = Vec::new();
                zip_file.read_to_end(&mut buf)?;
                out_file.write_all(&buf)?;
            }
        }
        Ok(())
    }
}
