use crate::library_db::LibraryDb;
use anyhow::{Context, Result};
use std::path::Path;

pub struct Organizer;

#[derive(sqlx::FromRow)]
struct BookRecord {
    current_path: String,
}

impl Organizer {
    /// Normaliza la ubicación de un libro en disco usando GutenCore::reorganize().
    /// Mueve el archivo a dest/Autor/Serie/Título.epub y actualiza la DB.
    pub async fn normalize_book(db: &LibraryDb, book_id: i64, base_dir: &Path) -> Result<()> {
        let pool = db.pool();
        
        let book_record = sqlx::query_as::<_, BookRecord>(
            "SELECT current_path FROM books WHERE id = ?"
        )
        .bind(book_id)
        .fetch_one(pool)
        .await
        .context("Libro no encontrado")?;

        let source_path = Path::new(&book_record.current_path);
        if !source_path.exists() {
            return Err(anyhow::anyhow!("Archivo origen no existe: {}", source_path.display()));
        }

        // Abrir con GutenCore y reorganizar
        let source = source_path.to_path_buf();
        let base = base_dir.to_path_buf();
        let target_path = tokio::task::spawn_blocking(move || {
            let mut core = gutencore::GutenCore::open_epub(&source)
                .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;
            core.reorganize(gutencore::FolderSchema::AuthorSeriesTitle, &base)
                .map_err(|e| anyhow::anyhow!("Reorganize error: {}", e))
        })
        .await??;

        // Actualizar db
        sqlx::query(
            "UPDATE books SET current_path = ?, is_normalized = 1 WHERE id = ?"
        )
        .bind(target_path.to_str().unwrap())
        .bind(book_id)
        .execute(pool)
        .await?;

        // Remover original si es distinto
        if source_path != target_path {
            let _ = tokio::fs::remove_file(source_path).await;
        }
        
        Ok(())
    }
}
