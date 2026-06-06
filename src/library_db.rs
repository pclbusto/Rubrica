use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author_id: i64,
    pub series_id: Option<i64>,
    pub original_path: String,
    pub current_path: String,
    pub is_normalized: bool,
    pub date_added: DateTime<Utc>,
    pub last_read: Option<DateTime<Utc>>,
    pub total_reading_time_secs: i64,
    pub reading_progress: Option<String>, // ID del bloque XHTML
}

#[derive(Clone)]
pub struct LibraryDb {
    pool: SqlitePool,
}

impl LibraryDb {
    pub async fn new(database_url: &str) -> Result<Self> {
        // sqlx 0.8 no crea el archivo automáticamente para URLs sqlite:///path absolutas.
        // Aseguramos que exista el archivo vacío antes de conectar.
        if database_url.starts_with("sqlite://") {
            let path = &database_url[9..]; // quitar "sqlite://"
            if !std::path::Path::new(path).exists() {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::File::create(path)?;
            }
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url).await?;
        
        Self::run_migrations(&pool).await?;

        let db = Self { pool };

        // Migración de fondo: computar hashes para libros existentes sin hash
        Self::backfill_hashes(&db.pool).await?;

        // Pre-cargar aliases de conveniencia si no existen
        let _ = db.set_alias("list", "books").await;
        let _ = db.set_alias("search", "books --fts").await;
        let _ = db.set_alias("ba", "books --author").await;
        let _ = db.set_alias("bs", "books --series").await;

        Ok(db)
    }

    /// Computa hashes SHA-256 para libros existentes que no tienen file_hash.
    async fn backfill_hashes(pool: &SqlitePool) -> Result<()> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, current_path FROM books WHERE file_hash IS NULL"
        )
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        println!("Calculando hashes para {} libros existentes...", rows.len());

        for (book_id, path) in rows {
            let hash = tokio::task::spawn_blocking(move || {
                let mut file = std::fs::File::open(&path).ok()?;
                let mut hasher = Sha256::new();
                std::io::copy(&mut file, &mut hasher).ok()?;
                Some(hex::encode(hasher.finalize()))
            })
            .await?;

            if let Some(h) = hash {
                sqlx::query("UPDATE books SET file_hash = ? WHERE id = ?")
                    .bind(&h)
                    .bind(book_id)
                    .execute(pool)
                    .await?;

                let _ = sqlx::query("INSERT INTO book_hashes (hash, book_id) VALUES (?, ?) ON CONFLICT DO NOTHING")
                    .bind(&h)
                    .bind(book_id)
                    .execute(pool)
                    .await;
            }
        }

        println!("Hashes calculados.");
        Ok(())
    }

    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS authors (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );"
        ).execute(pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS series (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );"
        ).execute(pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS books (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                author_id INTEGER NOT NULL REFERENCES authors(id),
                series_id INTEGER REFERENCES series(id),
                original_path TEXT NOT NULL,
                current_path TEXT NOT NULL,
                is_normalized BOOLEAN NOT NULL DEFAULT 0,
                date_added DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_read DATETIME,
                total_reading_time_secs INTEGER NOT NULL DEFAULT 0,
                reading_progress TEXT,
                file_hash TEXT
            );"
        ).execute(pool).await?;

        // Migración: agregar columna file_hash si la tabla ya existía sin ella
        let _ = sqlx::query("ALTER TABLE books ADD COLUMN file_hash TEXT")
            .execute(pool)
            .await;

        // Migración: agregar columnas de portada si no existen
        let _ = sqlx::query("ALTER TABLE books ADD COLUMN cover_href TEXT")
            .execute(pool)
            .await;
        let _ = sqlx::query("ALTER TABLE books ADD COLUMN cover_media_type TEXT")
            .execute(pool)
            .await;

        // Tabla FTS5 independiente para búsqueda de metadatos (título y autor).
        // No usa content='books' para evitar la complejidad de triggers;
        // el código gestiona inserciones/actualizaciones/borrados explícitamente.
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS books_fts USING fts5(title, author);"
        ).execute(pool).await?;

        // Tabla de aliases del CLI
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS aliases (
                name TEXT PRIMARY KEY,
                command TEXT NOT NULL
            );"
        ).execute(pool).await?;

        // Tablas de tags
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );"
        ).execute(pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS book_tags (
                book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
                tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
                PRIMARY KEY (book_id, tag_id)
            );"
        ).execute(pool).await?;

        // Tabla para historial de hashes
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS book_hashes (
                hash TEXT PRIMARY KEY,
                book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            );"
        ).execute(pool).await?;

        // Migración: popular book_hashes con los hashes existentes
        let _ = sqlx::query(
            "INSERT INTO book_hashes (hash, book_id)
             SELECT file_hash, id FROM books WHERE file_hash IS NOT NULL
             ON CONFLICT DO NOTHING"
        ).execute(pool).await;

        Ok(())
    }
    
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Inserta un documento en el índice FTS5.
    pub async fn insert_fts(&self, book_id: i64, title: &str, author: &str) -> Result<()> {
        sqlx::query("INSERT INTO books_fts (rowid, title, author) VALUES (?, ?, ?)")
            .bind(book_id)
            .bind(title)
            .bind(author)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Actualiza un documento en el índice FTS5.
    pub async fn update_fts(&self, book_id: i64, title: &str, author: &str) -> Result<()> {
        // FTS5 no soporta UPDATE directo; se borra y se reinserta.
        sqlx::query("DELETE FROM books_fts WHERE rowid = ?")
            .bind(book_id)
            .execute(&self.pool)
            .await?;
        self.insert_fts(book_id, title, author).await
    }

    /// Elimina un documento del índice FTS5.
    pub async fn delete_fts(&self, book_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM books_fts WHERE rowid = ?")
            .bind(book_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Lista todos los libros con nombre de autor y serie.
    pub async fn list_books(&self) -> Result<Vec<BookListItem>> {
        self.list_books_filtered(None, None, None, None, None).await
    }

    /// Lista libros aplicando filtros opcionales.
    pub async fn list_books_filtered(
        &self,
        normalized: Option<bool>,
        author: Option<&str>,
        series: Option<&str>,
        ids: Option<&[i64]>,
        tag: Option<&str>,
    ) -> Result<Vec<BookListItem>> {
        let mut conditions = Vec::new();
        let mut params: Vec<String> = Vec::new();

        if let Some(n) = normalized {
            conditions.push("b.is_normalized = ?".to_string());
            params.push(if n { "1".to_string() } else { "0".to_string() });
        }
        if let Some(a) = author {
            conditions.push("a.name LIKE ?".to_string());
            params.push(format!("%{}%", a));
        }
        if let Some(s) = series {
            conditions.push("s.name LIKE ?".to_string());
            params.push(format!("%{}%", s));
        }
        if let Some(t) = tag {
            conditions.push("EXISTS (SELECT 1 FROM book_tags bt2 JOIN tags t2 ON bt2.tag_id = t2.id WHERE bt2.book_id = b.id AND t2.name = ?)".to_string());
            params.push(t.to_string());
        }
        if let Some(id_list) = ids {
            if !id_list.is_empty() {
                let placeholders = id_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                conditions.push(format!("b.id IN ({})", placeholders));
            }
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT
                b.id,
                b.title,
                a.name as author_name,
                s.name as series_name,
                b.current_path,
                b.is_normalized,
                b.date_added,
                b.file_hash,
                b.cover_href,
                b.cover_media_type
            FROM books b
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            {}
            ORDER BY b.date_added DESC
            "#,
            where_clause
        );

        let mut q = sqlx::query_as::<_, BookListItem>(&query);
        for p in &params {
            q = q.bind(p);
        }
        if let Some(id_list) = ids {
            for id in id_list {
                q = q.bind(id);
            }
        }

        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Obtiene un libro por ID, incluyendo autor y serie.
    pub async fn get_book(&self, book_id: i64) -> Result<Option<BookDetail>> {
        let row = sqlx::query_as::<_, BookDetail>(
            r#"
            SELECT
                b.id,
                b.title,
                a.name as author_name,
                s.name as series_name,
                b.current_path,
                b.date_added,
                b.cover_href,
                b.cover_media_type
            FROM books b
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            WHERE b.id = ?
            "#
        )
        .bind(book_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Lista todos los autores con cantidad de libros.
    pub async fn list_authors(&self) -> Result<Vec<AuthorStats>> {
        let rows = sqlx::query_as::<_, AuthorStats>(
            r#"
            SELECT
                a.id,
                a.name,
                COUNT(b.id) as book_count
            FROM authors a
            LEFT JOIN books b ON a.id = b.author_id
            GROUP BY a.id, a.name
            ORDER BY book_count DESC, a.name ASC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Lista todas las series con cantidad de libros.
    pub async fn list_series(&self) -> Result<Vec<SeriesStats>> {
        let rows = sqlx::query_as::<_, SeriesStats>(
            r#"
            SELECT
                s.id,
                s.name,
                COUNT(b.id) as book_count
            FROM series s
            LEFT JOIN books b ON s.id = b.series_id
            GROUP BY s.id, s.name
            ORDER BY book_count DESC, s.name ASC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Lista libros de un autor específico (búsqueda exacta o parcial).
    pub async fn list_books_by_author(&self, author_name: &str) -> Result<Vec<BookListItem>> {
        let rows = sqlx::query_as::<_, BookListItem>(
            r#"
            SELECT
                b.id,
                b.title,
                a.name as author_name,
                s.name as series_name,
                b.current_path,
                b.is_normalized,
                b.date_added,
                b.file_hash,
                b.cover_href,
                b.cover_media_type
            FROM books b
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            WHERE a.name LIKE ?
            ORDER BY b.title ASC
            "#
        )
        .bind(format!("%{}%", author_name))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Busca libros en el índice FTS5 por título o autor.
    pub async fn search_fts(&self, query: &str) -> Result<Vec<BookListItem>> {
        let rows = sqlx::query_as::<_, BookListItem>(
            r#"
            SELECT
                b.id,
                b.title,
                a.name as author_name,
                s.name as series_name,
                b.current_path,
                b.is_normalized,
                b.date_added,
                b.file_hash,
                b.cover_href,
                b.cover_media_type
            FROM books b
            JOIN books_fts f ON b.id = f.rowid
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            WHERE books_fts MATCH ?
            ORDER BY b.date_added DESC
            "#
        )
        .bind(query)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Alias CRUD ───────────────────────────────────────────────────

    /// Obtiene el comando correspondiente a un alias.
    pub async fn get_alias(&self, name: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT command FROM aliases WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    /// Crea o actualiza un alias.
    pub async fn set_alias(&self, name: &str, command: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO aliases (name, command) VALUES (?, ?)
             ON CONFLICT(name) DO UPDATE SET command = excluded.command"
        )
        .bind(name)
        .bind(command)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Elimina un alias.
    pub async fn delete_alias(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM aliases WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Lista todos los aliases.
    pub async fn list_aliases(&self) -> Result<Vec<Alias>> {
        let rows = sqlx::query_as::<_, Alias>("SELECT name, command FROM aliases ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Busca un libro por su hash de archivo (actual o historial). Devuelve el ID si existe.
    pub async fn find_by_hash(&self, hash: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT book_id FROM book_hashes WHERE hash = ?
             UNION
             SELECT id FROM books WHERE file_hash = ?"
        )
            .bind(hash)
            .bind(hash)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    /// Añade un hash al historial de un libro.
    pub async fn add_book_hash(&self, book_id: i64, hash: &str) -> Result<()> {
        sqlx::query("INSERT INTO book_hashes (hash, book_id) VALUES (?, ?) ON CONFLICT DO NOTHING")
            .bind(hash)
            .bind(book_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Elimina un libro de la base de datos. Si `delete_file` es true,
    /// también borra el archivo físico del disco.
    /// Devuelve la ruta del archivo borrado, o None si no se encontró.
    /// Crea una nueva serie vacía.
    pub async fn add_series(&self, name: &str) -> Result<i64> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO series (name) VALUES (?) RETURNING id"
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Borra una serie. Si `unlink` es true, desvincula los libros asociados primero.
    /// Si es false y la serie tiene libros, devuelve error.
    pub async fn delete_series(&self, series_id: i64, unlink: bool) -> Result<()> {
        let pool = &self.pool;

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM books WHERE series_id = ?"
        )
        .bind(series_id)
        .fetch_one(pool)
        .await?;

        if count.0 > 0 {
            if unlink {
                sqlx::query("UPDATE books SET series_id = NULL WHERE series_id = ?")
                    .bind(series_id)
                    .execute(pool)
                    .await?;
            } else {
                return Err(anyhow::anyhow!(
                    "La serie tiene {} libro(s) asociado(s). Usá --force para desvincularlos primero.",
                    count.0
                ));
            }
        }

        sqlx::query("DELETE FROM series WHERE id = ?")
            .bind(series_id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Asigna un libro a una serie.
    pub async fn assign_book_series(&self, book_id: i64, series_id: i64) -> Result<()> {
        sqlx::query("UPDATE books SET series_id = ? WHERE id = ?")
            .bind(series_id)
            .bind(book_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Desvincula un libro de su serie (series_id = NULL).
    pub async fn unassign_book_series(&self, book_id: i64) -> Result<()> {
        sqlx::query("UPDATE books SET series_id = NULL WHERE id = ?")
            .bind(book_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Renombra una serie.
    pub async fn rename_series(&self, series_id: i64, name: &str) -> Result<()> {
        sqlx::query("UPDATE series SET name = ? WHERE id = ?")
            .bind(name)
            .bind(series_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Actualiza metadatos de un libro (título, autor, serie).
    /// Si author_id es None, busca o crea el autor por nombre.
    pub async fn update_book(&self, book_id: i64, title: Option<&str>, author_id: Option<i64>, author_name: Option<&str>, series_id: Option<i64>) -> Result<()> {
        let pool = &self.pool;

        let mut author_id = author_id;
        if let Some(name) = author_name {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO authors (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name=name RETURNING id"
            )
            .bind(name)
            .fetch_one(pool)
            .await?;
            author_id = Some(id);
        }

        let mut updates = Vec::new();
        let mut params: Vec<String> = Vec::new();

        if let Some(t) = title {
            updates.push("title = ?");
            params.push(t.to_string());
        }
        if let Some(aid) = author_id {
            updates.push("author_id = ?");
            params.push(aid.to_string());
        }
        if let Some(sid) = series_id {
            updates.push("series_id = ?");
            params.push(sid.to_string());
        }

        if !updates.is_empty() {
            let sql = format!("UPDATE books SET {} WHERE id = ?", updates.join(", "));
            let mut q = sqlx::query(&sql);
            for p in &params {
                q = q.bind(p);
            }
            q = q.bind(book_id);
            q.execute(pool).await?;
        }

        Ok(())
    }

    // ─── Tags ─────────────────────────────────────────────────────────

    /// Crea un nuevo tag.
    pub async fn create_tag(&self, name: &str) -> Result<i64> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO tags (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name=name RETURNING id"
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Lista todos los tags con cantidad de libros.
    pub async fn list_tags(&self) -> Result<Vec<TagStats>> {
        let rows = sqlx::query_as::<_, TagStats>(
            r#"
            SELECT
                t.id,
                t.name,
                COUNT(bt.book_id) as book_count
            FROM tags t
            LEFT JOIN book_tags bt ON t.id = bt.tag_id
            GROUP BY t.id, t.name
            ORDER BY book_count DESC, t.name ASC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Asigna un tag a un libro (crea el tag si no existe).
    pub async fn add_tag_to_book(&self, book_id: i64, tag_name: &str) -> Result<()> {
        let tag_id: i64 = sqlx::query_scalar(
            "INSERT INTO tags (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name=name RETURNING id"
        )
        .bind(tag_name)
        .fetch_one(&self.pool)
        .await?;

        let _ = sqlx::query("INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?, ?)")
            .bind(book_id)
            .bind(tag_id)
            .execute(&self.pool)
            .await;

        Ok(())
    }

    /// Desvincula un tag de un libro.
    pub async fn remove_tag_from_book(&self, book_id: i64, tag_name: &str) -> Result<()> {
        let tag_id: Option<(i64,)> = sqlx::query_as("SELECT id FROM tags WHERE name = ?")
            .bind(tag_name)
            .fetch_optional(&self.pool)
            .await?;

        if let Some((id,)) = tag_id {
            sqlx::query("DELETE FROM book_tags WHERE book_id = ? AND tag_id = ?")
                .bind(book_id)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    // ─── OPDS queries ───────────────────────────────────────────────────

    /// Lista libros de un autor específico por ID.
    pub async fn list_books_by_author_id(&self, author_id: i64) -> Result<Vec<BookListItem>> {
        let rows = sqlx::query_as::<_, BookListItem>(
            r#"
            SELECT
                b.id,
                b.title,
                a.name as author_name,
                s.name as series_name,
                b.current_path,
                b.is_normalized,
                b.date_added,
                b.file_hash,
                b.cover_href,
                b.cover_media_type
            FROM books b
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            WHERE b.author_id = ?
            ORDER BY b.title ASC
            "#
        )
        .bind(author_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Lista libros de una serie específica por ID.
    pub async fn list_books_by_series_id(&self, series_id: i64) -> Result<Vec<BookListItem>> {
        let rows = sqlx::query_as::<_, BookListItem>(
            r#"
            SELECT
                b.id,
                b.title,
                a.name as author_name,
                s.name as series_name,
                b.current_path,
                b.is_normalized,
                b.date_added,
                b.file_hash,
                b.cover_href,
                b.cover_media_type
            FROM books b
            JOIN authors a ON b.author_id = a.id
            LEFT JOIN series s ON b.series_id = s.id
            WHERE b.series_id = ?
            ORDER BY b.title ASC
            "#
        )
        .bind(series_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_book(&self, book_id: i64, delete_file: bool) -> Result<Option<String>> {
        let pool = &self.pool;

        // 1. Obtener la ruta física antes de borrar el registro
        let path_opt: Option<(String,)> = sqlx::query_as("SELECT current_path FROM books WHERE id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await?;

        let file_path = match path_opt {
            Some(p) => p.0,
            None => return Ok(None), // Libro no existe
        };

        // 2. Eliminar del índice FTS5
        sqlx::query("DELETE FROM books_fts WHERE rowid = ?")
            .bind(book_id)
            .execute(pool)
            .await?;

        // 3. Eliminar de la tabla books
        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(book_id)
            .execute(pool)
            .await?;

        // 4. Borrar archivo físico si se pidió
        if delete_file {
            if std::path::Path::new(&file_path).exists() {
                tokio::fs::remove_file(&file_path).await?;
                Ok(Some(file_path))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

#[derive(sqlx::FromRow, Debug)]
pub struct BookListItem {
    pub id: i64,
    pub title: String,
    pub author_name: String,
    pub series_name: Option<String>,
    pub current_path: String,
    pub is_normalized: bool,
    pub date_added: DateTime<Utc>,
    pub file_hash: Option<String>,
    pub cover_href: Option<String>,
    pub cover_media_type: Option<String>,
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct BookDetail {
    pub id: i64,
    pub title: String,
    pub author_name: String,
    pub series_name: Option<String>,
    pub current_path: String,
    pub date_added: DateTime<Utc>,
    pub cover_href: Option<String>,
    pub cover_media_type: Option<String>,
}

#[derive(sqlx::FromRow, Debug)]
pub struct Alias {
    pub name: String,
    pub command: String,
}

#[derive(sqlx::FromRow, Debug)]
pub struct AuthorStats {
    pub id: i64,
    pub name: String,
    pub book_count: i64,
}

#[derive(sqlx::FromRow, Debug)]
pub struct SeriesStats {
    pub id: i64,
    pub name: String,
    pub book_count: i64,
}

#[derive(sqlx::FromRow, Debug)]
pub struct TagStats {
    pub id: i64,
    pub name: String,
    pub book_count: i64,
}
