use crate::library_db::LibraryDb;
use crate::gutencore::GutenAdapter;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct Organizer;

#[derive(sqlx::FromRow)]
struct BookRecord {
    current_path: String,
    title: String,
    author_name: String,
    series_name: Option<String>,
}

impl Organizer {
    /// Sanitizador de nombres de archivo
    pub fn sanitize_filename(name: &str) -> String {
        let cleaned = name.trim().replace(' ', "_");
        sanitize_filename::sanitize(cleaned)
    }

    /// Resolución de directorios inteligente
    pub fn resolve_directory(base_dir: &Path, author: &str, series: Option<&str>, title: &str) -> PathBuf {
        let clean_author = Self::sanitize_filename(author);
        let clean_title = Self::sanitize_filename(title);
        
        let mut path = base_dir.join(clean_author);
        if let Some(s) = series {
            path = path.join(Self::sanitize_filename(s));
        }
        
        path.join(format!("{}.epub", clean_title))
    }

    /// Transacción segura de archivos de forma asíncrona
    pub async fn normalize_book(db: &LibraryDb, book_id: i64, base_dir: &Path) -> Result<()> {
        let pool = db.pool();
        
        // Obtener datos del libro
        let book_record = sqlx::query_as::<_, BookRecord>(
            r#"
            SELECT b.current_path, b.title, a.name as author_name, s.name as series_name
            FROM books b
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            WHERE b.id = ?
            "#
        )
        .bind(book_id)
        .fetch_one(pool)
        .await
        .context("Libro no encontrado")?;

        let source_path = Path::new(&book_record.current_path);
        if !source_path.exists() {
            return Err(anyhow::anyhow!("Archivo origen no existe: {}", source_path.display()));
        }

        // Generar nueva ruta
        let target_path = Self::resolve_directory(
            base_dir,
            &book_record.author_name,
            book_record.series_name.as_deref(),
            &book_record.title
        );

        if let Some(parent) = target_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Copiar el archivo
        tokio::fs::copy(source_path, &target_path).await?;

        // Validar la copia (Test de lectura rápido)
        match GutenAdapter::open_folder(target_path.to_str().unwrap()).await {
            Ok(_) => {
                // Actualizar db
                sqlx::query(
                    "UPDATE books SET current_path = ?, is_normalized = 1 WHERE id = ?"
                )
                .bind(target_path.to_str().unwrap())
                .bind(book_id)
                .execute(pool)
                .await?;

                // Remover original si es distinto a current_path y se ha normalizado correctamente
                if source_path != target_path {
                    let _ = tokio::fs::remove_file(source_path).await; // Ignorar errores de borrado
                }
                
                Ok(())
            }
            Err(e) => {
                // Rollback de la copia
                let _ = tokio::fs::remove_file(&target_path).await;
                Err(anyhow::anyhow!("Fallo validación pos-copia: {}", e))
            }
        }
    }
}
