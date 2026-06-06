use crate::library_db::LibraryDb;
use anyhow::{Context, Result};
use tokio::sync::mpsc;

/// Metadatos extraídos de un EPUB para la pipeline de importación.
#[derive(Debug, Clone)]
struct EpubMeta {
    title: String,
    author: String,
    series: Option<String>,
    progress: Option<String>,
    cover_href: Option<String>,
    cover_media_type: Option<String>,
}

/// Resultado de una operación de importación individual.
#[derive(Debug, Clone)]
pub enum ImportStatus {
    /// El libro fue importado exitosamente.
    Imported,
    /// El libro ya existía en la biblioteca (duplicado por hash).
    Duplicate { existing_id: i64 },
}

pub struct Pipeline;

impl Pipeline {
    /// Importa un archivo de forma asíncrona, validándolo y actualizando la DB e índice FTS5.
    /// Devuelve `ImportStatus::Imported` si es nuevo, o `ImportStatus::Duplicate` si ya existe.
    pub async fn import_file(db: &LibraryDb, original_path: String) -> Result<ImportStatus> {
        let path = original_path.clone();

        // Abrir EPUB con GutenCore (calcula hash y extrae metadatos nativamente)
        let (file_hash, metadata) = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, EpubMeta)> {
            let core = gutencore::GutenCore::open_epub(&path)
                .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;

            let hash = core.file_hash.clone()
                .context("Hash not computed by GutenCore")?
                .to_string();

            let md = core.get_metadata()
                .context("No metadata loaded by GutenCore")?;

            // Extraer metadatos rubrica:* del OPF (GutenCore no los lee)
            let mut series = md.series.clone();
            let mut progress = None;

            if let Some(opf_path) = &core.opf_path {
                if let Ok(opf_content) = std::fs::read_to_string(opf_path) {
                    if let Ok(doc) = roxmltree::Document::parse(&opf_content) {
                        let ns_opf = "http://www.idpf.org/2007/opf";
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
                    }
                }
            }

            // Extraer portada
            let cover = core.get_cover_image();
            let cover_href = cover.map(|c| c.href.clone());
            let cover_media_type = cover.map(|c| c.media_type.clone());

            Ok((hash, EpubMeta {
                title: md.title.clone(),
                author: md.author.clone().unwrap_or_else(|| "Unknown".to_string()),
                series,
                progress,
                cover_href,
                cover_media_type,
            }))
        })
        .await??;

        // 0. Detectar duplicados
        if let Some(existing_id) = db.find_by_hash(&file_hash).await? {
            return Ok(ImportStatus::Duplicate { existing_id });
        }

        let pool = db.pool();

        // 1. Insertar o recuperar autor
        let author_id: i64 = sqlx::query_scalar(
            "INSERT INTO authors (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name=name RETURNING id"
        )
        .bind(&metadata.author)
        .fetch_one(pool)
        .await?;

        // 2. Insertar o recuperar serie (si existe)
        let series_id: Option<i64> = if let Some(series_name) = metadata.series {
            Some(sqlx::query_scalar(
                "INSERT INTO series (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name=name RETURNING id"
            )
            .bind(&series_name)
            .fetch_one(pool)
            .await?)
        } else {
            None
        };

        // 3. Registrar libro (con hash)
        let current_path = original_path.clone();
        let is_normalized = false;

        let book_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO books (title, author_id, series_id, original_path, current_path, is_normalized, reading_progress, file_hash, cover_href, cover_media_type)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id
            "#
        )
        .bind(&metadata.title)
        .bind(author_id)
        .bind(series_id)
        .bind(&original_path)
        .bind(&current_path)
        .bind(is_normalized)
        .bind(&metadata.progress)
        .bind(&file_hash)
        .bind(&metadata.cover_href)
        .bind(&metadata.cover_media_type)
        .fetch_one(pool)
        .await?;

        // 4. Guardar hash en el historial
        db.add_book_hash(book_id, &file_hash).await?;

        // 5. Indexación FTS5 Global (solo metadatos: título y autor)
        db.insert_fts(book_id, &metadata.title, &metadata.author).await?;
        
        Ok(ImportStatus::Imported)
    }

    /// Importación por lotes no bloqueante utilizando canales de mensajes.
    /// Cada mensaje contiene el path y el resultado de la importación.
    pub fn import_batch(
        db: LibraryDb,
        paths: Vec<String>,
    ) -> mpsc::Receiver<(String, Result<ImportStatus>)> {
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            for path in paths {
                let result = Self::import_file(&db, path.clone()).await;
                if tx.send((path, result)).await.is_err() {
                    break; // Receptor cerrado
                }
            }
        });

        rx
    }
}
