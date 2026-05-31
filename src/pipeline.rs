use crate::library_db::LibraryDb;
use crate::gutencore::GutenAdapter;
use anyhow::Result;
use sha2::{Sha256, Digest};
use tokio::sync::mpsc;

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
        // 0. Calcular hash SHA-256 del archivo para detectar duplicados
        let file_hash = Self::compute_file_hash(&original_path).await?;

        if let Some(existing_id) = db.find_by_hash(&file_hash).await? {
            return Ok(ImportStatus::Duplicate { existing_id });
        }

        // 1. Integración con GutenCore (Validación de contenedor y OPF) y extracción dual de metadatos
        let metadata = GutenAdapter::validate_epub(&original_path).await?;
        
        let pool = db.pool();

        // 2. Insertar o recuperar autor
        let author_id: i64 = sqlx::query_scalar(
            "INSERT INTO authors (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name=name RETURNING id"
        )
        .bind(&metadata.author)
        .fetch_one(pool)
        .await?;

        // 3. Insertar o recuperar serie (si existe)
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

        // 4. Registrar libro (con hash)
        let current_path = original_path.clone();
        let is_normalized = false;

        let book_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO books (title, author_id, series_id, original_path, current_path, is_normalized, reading_progress, file_hash)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
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
        .fetch_one(pool)
        .await?;

        // 5. Indexación FTS5 Global (solo metadatos: título y autor)
        db.insert_fts(book_id, &metadata.title, &metadata.author).await?;
        
        Ok(ImportStatus::Imported)
    }

    /// Computa el hash SHA-256 de un archivo de forma asíncrona.
    async fn compute_file_hash(path: &str) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::File::open(&path)?;
            let mut hasher = Sha256::new();
            std::io::copy(&mut file, &mut hasher)?;
            let result = hasher.finalize();
            Ok(hex::encode(result))
        })
        .await?
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
