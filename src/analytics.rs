use crate::library_db::LibraryDb;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LibraryMetrics {
    pub total_books: i64,
    pub total_reading_time_secs: i64,
}

#[derive(Debug, Serialize)]
pub struct BookHealthReport {
    pub book_id: i64,
    pub broken_links: u32,
    pub orphan_anchors: u32,
    pub css_discrepancies: u32,
}

pub struct Analytics;

#[derive(sqlx::FromRow)]
struct MetricsRow {
    total_books: i64,
    total_time: Option<i64>,
}

impl Analytics {
    /// Contador de métricas rápido a nivel de base de datos
    pub async fn get_global_metrics(db: &LibraryDb) -> Result<LibraryMetrics> {
        let pool = db.pool();
        
        let metrics = sqlx::query_as::<_, MetricsRow>(
            r#"
            SELECT 
                COUNT(*) as total_books,
                SUM(total_reading_time_secs) as total_time
            FROM books
            "#
        )
        .fetch_one(pool)
        .await?;

        Ok(LibraryMetrics {
            total_books: metrics.total_books as i64,
            total_reading_time_secs: metrics.total_time.unwrap_or(0) as i64,
        })
    }

    /// Módulo de salud editorial agnóstico.
    /// Abre el EPUB con GutenCore y ejecuta validate_links() en un thread bloqueante.
    pub async fn validate_links(book_id: i64, current_path: &str) -> Result<BookHealthReport> {
        let path = current_path.to_string();
        let broken = tokio::task::spawn_blocking(move || {
            let core = gutencore::GutenCore::open_epub(&path)
                .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;
            core.validate_links()
                .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))
        })
        .await??;

        Ok(BookHealthReport {
            book_id,
            broken_links: broken.len() as u32,
            orphan_anchors: 0,
            css_discrepancies: 0,
        })
    }
}
