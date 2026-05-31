use crate::library_db::{LibraryDb, BookListItem};
use anyhow::Result;
use axum::{
    extract::{Path as AxumPath, State},
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
    body::Body,
};
use std::net::SocketAddr;
use std::path::Path as StdPath;

pub struct SyncSubsystem;

impl SyncSubsystem {
    /// Abstracción de dispositivo MTP
    pub async fn sync_to_mtp(device_mount_path: &StdPath, books_to_sync: Vec<&StdPath>) -> Result<()> {
        let books_dir = device_mount_path.join("Books");
        if !books_dir.exists() {
            return Err(anyhow::anyhow!("Directorio /Books no encontrado en el dispositivo MTP"));
        }
        for book in books_to_sync {
            if let Some(filename) = book.file_name() {
                let target = books_dir.join(filename);
                tokio::fs::copy(book, target).await?;
            }
        }
        Ok(())
    }

    /// Inicia el Servidor OPDS Embebido con feed dinámico.
    pub async fn start_opds_server(db: LibraryDb, port: u16) -> Result<()> {
        let app = Router::new()
            .route("/opds", get(opds_feed))
            .route("/opds/download/:id", get(opds_download))
            .with_state(db);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Ok(())
    }
}

/// Genera el feed OPDS/Atom con todos los libros de la biblioteca.
async fn opds_feed(State(db): State<LibraryDb>) -> Response {
    let books = match db.list_books().await {
        Ok(b) => b,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e));
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut entries = String::new();

    for book in &books {
        entries.push_str(&opds_entry(book, &now));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog">
  <id>urn:uuid:rubrica-library</id>
  <title>Biblioteca Rúbrica</title>
  <updated>{now}</updated>
  <author><name>Rúbrica Core</name></author>
  <link rel="self" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
{entries}
</feed>"#
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/atom+xml;profile=opds-catalog;kind=acquisition")
        .body(Body::from(xml))
        .unwrap()
}

/// Sirve un archivo EPUB directamente para descarga.
async fn opds_download(
    State(db): State<LibraryDb>,
    AxumPath(book_id): AxumPath<i64>,
) -> Response {
    let book = match db.get_book(book_id).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Libro no encontrado");
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e));
        }
    };

    let data = match tokio::fs::read(&book.current_path).await {
        Ok(d) => d,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error leyendo archivo: {}", e));
        }
    };

    let filename = std::path::Path::new(&book.current_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("book.epub");

    let disposition = format!(r#"attachment; filename="{}""#, filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/epub+zip")
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(data))
        .unwrap()
}

fn opds_entry(book: &BookListItem, updated: &str) -> String {
    let title = xml_escape(&book.title);
    let author = xml_escape(&book.author_name);
    let series = book.series_name.as_deref().unwrap_or("");
    let series_tag = if series.is_empty() {
        String::new()
    } else {
        format!("  <dc:publisher>{}</dc:publisher>\n", xml_escape(series))
    };

    format!(
        r#"  <entry>
    <title>{title}</title>
    <id>urn:rubrica:book:{id}</id>
    <author><name>{author}</name></author>
    <updated>{updated}</updated>
{series_tag}    <link rel="http://opds-spec.org/acquisition" type="application/epub+zip" href="/opds/download/{id}" title="Descargar EPUB"/>
  </entry>
"#,
        title = title,
        id = book.id,
        author = author,
        updated = updated,
        series_tag = series_tag,
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn error_response(status: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(msg.to_string()))
        .unwrap()
}
