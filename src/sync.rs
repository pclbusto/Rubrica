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
            .route("/opds/authors", get(opds_authors))
            .route("/opds/author/:id", get(opds_author_books))
            .route("/opds/series", get(opds_series))
            .route("/opds/series/:id", get(opds_series_books))
            .route("/opds/cover/:id", get(opds_cover))
            .route("/opds/browser", get(opds_browser))
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

/// Feed de autores para navegación OPDS.
async fn opds_authors(State(db): State<LibraryDb>) -> Response {
    let authors = match db.list_authors().await {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e)),
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut entries = String::new();

    for author in &authors {
        let name = xml_escape(&author.name);
        entries.push_str(&format!(
            r#"  <entry>
    <title>{name}</title>
    <id>urn:rubrica:author:{id}</id>
    <updated>{now}</updated>
    <link rel="subsection" type="application/atom+xml;profile=opds-catalog;kind=acquisition" href="/opds/author/{id}" title="Libros de {name}"/>
  </entry>
"#,
            name = name,
            id = author.id,
            now = now,
        ));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog">
  <id>urn:uuid:rubrica-authors</id>
  <title>Autores</title>
  <updated>{now}</updated>
  <author><name>Rúbrica Core</name></author>
  <link rel="self" href="/opds/authors" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="start" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
{entries}
</feed>"#,
        now = now,
        entries = entries
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/atom+xml;profile=opds-catalog;kind=acquisition")
        .body(Body::from(xml))
        .unwrap()
}

/// Feed de libros de un autor específico.
async fn opds_author_books(State(db): State<LibraryDb>, AxumPath(author_id): AxumPath<i64>) -> Response {
    let books = match db.list_books_by_author_id(author_id).await {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e)),
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut entries = String::new();

    for book in &books {
        entries.push_str(&opds_entry(book, &now));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog">
  <id>urn:rubrica:author:{author_id}</id>
  <title>Libros del autor</title>
  <updated>{now}</updated>
  <author><name>Rúbrica Core</name></author>
  <link rel="self" href="/opds/author/{author_id}" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="start" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="up" href="/opds/authors" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
{entries}
</feed>"#,
        author_id = author_id,
        now = now,
        entries = entries
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/atom+xml;profile=opds-catalog;kind=acquisition")
        .body(Body::from(xml))
        .unwrap()
}

/// Feed de series para navegación OPDS.
async fn opds_series(State(db): State<LibraryDb>) -> Response {
    let series_list = match db.list_series().await {
        Ok(s) => s,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e)),
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut entries = String::new();

    for s in &series_list {
        let name = xml_escape(&s.name);
        entries.push_str(&format!(
            r#"  <entry>
    <title>{name}</title>
    <id>urn:rubrica:series:{id}</id>
    <updated>{now}</updated>
    <link rel="subsection" type="application/atom+xml;profile=opds-catalog;kind=acquisition" href="/opds/series/{id}" title="Libros de {name}"/>
  </entry>
"#,
            name = name,
            id = s.id,
            now = now,
        ));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog">
  <id>urn:uuid:rubrica-series</id>
  <title>Series</title>
  <updated>{now}</updated>
  <author><name>Rúbrica Core</name></author>
  <link rel="self" href="/opds/series" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="start" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
{entries}
</feed>"#,
        now = now,
        entries = entries
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/atom+xml;profile=opds-catalog;kind=acquisition")
        .body(Body::from(xml))
        .unwrap()
}

/// Feed de libros de una serie específica.
async fn opds_series_books(State(db): State<LibraryDb>, AxumPath(series_id): AxumPath<i64>) -> Response {
    let books = match db.list_books_by_series_id(series_id).await {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e)),
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut entries = String::new();

    for book in &books {
        entries.push_str(&opds_entry(book, &now));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog">
  <id>urn:rubrica:series:{series_id}</id>
  <title>Libros de la serie</title>
  <updated>{now}</updated>
  <author><name>Rúbrica Core</name></author>
  <link rel="self" href="/opds/series/{series_id}" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="start" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="up" href="/opds/series" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
{entries}
</feed>"#,
        series_id = series_id,
        now = now,
        entries = entries
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

    let cover_links = if book.cover_href.is_some() {
        let mt = book.cover_media_type.as_deref().unwrap_or("image/jpeg");
        format!(
            r#"    <link rel="http://opds-spec.org/image" href="/opds/cover/{id}" type="{mt}"/>
    <link rel="http://opds-spec.org/image/thumbnail" href="/opds/cover/{id}" type="{mt}"/>
"#,
            id = book.id,
            mt = mt,
        )
    } else {
        String::new()
    };

    format!(
        r#"  <entry>
    <title>{title}</title>
    <id>urn:rubrica:book:{id}</id>
    <author><name>{author}</name></author>
    <updated>{updated}</updated>
{series_tag}{cover_links}    <link rel="http://opds-spec.org/acquisition" type="application/epub+zip" href="/opds/download/{id}" title="Descargar EPUB"/>
  </entry>
"#,
        title = title,
        id = book.id,
        author = author,
        updated = updated,
        series_tag = series_tag,
        cover_links = cover_links,
    )
}

/// Sirve la imagen de portada de un libro directamente desde el EPUB.
async fn opds_cover(State(db): State<LibraryDb>, AxumPath(book_id): AxumPath<i64>) -> Response {
    let book = match db.get_book(book_id).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Libro no encontrado");
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e));
        }
    };

    let (Some(href), Some(media_type)) = (book.cover_href, book.cover_media_type) else {
        return error_response(StatusCode::NOT_FOUND, "Libro sin portada");
    };

    let path = book.current_path;
    let image_data = match tokio::task::spawn_blocking(move || {
        let core = gutencore::GutenCore::open_epub(&path)?;
        let opf_dir = core.opf_dir.as_ref().ok_or_else(|| {
            anyhow::anyhow!("OPF dir not set")
        })?;
        let abs_path = opf_dir.join(&href);
        std::fs::read(&abs_path).map_err(|e| anyhow::anyhow!("Error leyendo imagen: {}", e))
    }).await {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{}", e));
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Task error: {}", e));
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, media_type)
        .body(Body::from(image_data))
        .unwrap()
}

/// Página HTML navegable para ver la biblioteca desde un navegador web.
async fn opds_browser(State(db): State<LibraryDb>) -> Response {
    let books = match db.list_books().await {
        Ok(b) => b,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Database error: {}", e));
        }
    };

    let mut cards = String::new();
    for book in &books {
        let title = xml_escape(&book.title);
        let author = xml_escape(&book.author_name);
        let series = book.series_name.as_deref().map(xml_escape).unwrap_or_default();
        let series_html = if series.is_empty() {
            String::new()
        } else {
            format!(r#"<div class="series">{}</div>"#, series)
        };

        let cover_html = if book.cover_href.is_some() {
            format!(
                r#"<img src="/opds/cover/{id}" alt="{title}" loading="lazy"/>"#,
                id = book.id,
                title = title,
            )
        } else {
            r#"<div class="no-cover">Sin portada</div>"#.to_string()
        };

        cards.push_str(&format!(
            r#"<div class="card">
  <div class="cover">{cover}</div>
  <div class="info">
    <div class="title">{title}</div>
    <div class="author">{author}</div>
    {series}
    <a class="download" href="/opds/download/{id}" download>Descargar EPUB</a>
  </div>
</div>
"#,
            cover = cover_html,
            title = title,
            author = author,
            series = series_html,
            id = book.id,
        ));
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="es">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Biblioteca Rúbrica</title>
<style>
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; padding: 1rem;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #121212; color: #e0e0e0;
  }}
  h1 {{ text-align: center; margin-bottom: 1.5rem; color: #fff; }}
  .grid {{
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 1rem;
    max-width: 1200px;
    margin: 0 auto;
  }}
  .card {{
    background: #1e1e1e;
    border-radius: 8px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    box-shadow: 0 2px 8px rgba(0,0,0,0.4);
  }}
  .cover {{
    width: 100%;
    aspect-ratio: 2 / 3;
    background: #333;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }}
  .cover img {{ width: 100%; height: 100%; object-fit: cover; display: block; }}
  .no-cover {{ color: #888; font-size: 0.85rem; }}
  .info {{
    padding: 0.75rem;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }}
  .title {{ font-weight: 600; font-size: 0.95rem; color: #fff; line-height: 1.2; }}
  .author {{ font-size: 0.8rem; color: #bbb; }}
  .series {{ font-size: 0.75rem; color: #888; font-style: italic; }}
  .download {{
    margin-top: auto;
    padding-top: 0.5rem;
    text-align: center;
    background: #2a2a2a;
    color: #81c784;
    text-decoration: none;
    border-radius: 4px;
    padding: 0.5rem;
    font-size: 0.8rem;
    font-weight: 500;
  }}
  .download:hover {{ background: #333; }}
  .stats {{ text-align: center; margin-top: 1.5rem; color: #888; font-size: 0.8rem; }}
</style>
</head>
<body>
<h1>Biblioteca Rúbrica</h1>
<div class="grid">
{cards}
</div>
<div class="stats">{count} libros</div>
</body>
</html>"#,
        cards = cards,
        count = books.len(),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap()
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
