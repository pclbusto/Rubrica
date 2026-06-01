use anyhow::Result;
use clap::{Parser, Subcommand};
use rubrica::{Analytics, LibraryDb, Organizer, Pipeline, SyncSubsystem};
use rubrica::pipeline::ImportStatus;
use rustyline::completion::{Completer, Pair};
use rustyline::{Config, Editor};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rubrica")]
#[command(about = "Rúbrica CLI - Administrador de bibliotecas EPUB")]
#[command(arg_required_else_help = false)]
struct Cli {
    /// Ruta a la base de datos SQLite de Rúbrica
    #[arg(short, long, default_value = "library.db")]
    db: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Clone)]
enum Commands {
    /// Inicializa la base de datos
    Init,
    /// Importa un archivo EPUB a la biblioteca
    Import {
        /// Ruta al archivo .epub
        path: PathBuf,
    },
    /// Importa recursivamente todos los EPUBs de un directorio
    ImportDir {
        /// Ruta al directorio
        path: PathBuf,
    },
    /// Lista libros de la biblioteca
    Books {
        /// Muestra detalle completo de cada libro (formato vertical)
        #[arg(short, long)]
        long: bool,
        /// Muestra información avanzada del EPUB (capítulos, tamaño, etc.)
        #[arg(short, long)]
        extralong: bool,
        /// Filtrar solo libros normalizados
        #[arg(short, long)]
        normalized: bool,
        /// Filtrar por autor (búsqueda parcial)
        #[arg(short, long)]
        author: Option<String>,
        /// Filtrar por serie (búsqueda parcial)
        #[arg(short, long)]
        series: Option<String>,
        /// Buscar en título y autor (FTS5).
        /// Sintaxis: palabra1 palabra2 (AND implícito), -palabra (NOT), +palabra (obligatorio), "frase exacta"
        #[arg(short, long)]
        fts: Option<String>,
        /// Filtrar por IDs específicos (coma separado, ej: 1,5,10)
        #[arg(short, long, value_delimiter = ',')]
        ids: Option<Vec<i64>>,
    },
    /// Normaliza la ubicación de un libro en disco
    Normalize {
        /// ID del libro en la base de datos
        book_id: i64,
        /// Directorio base donde reorganizar los archivos
        #[arg(short, long, default_value = "~/Books/Rubrica")]
        base_dir: PathBuf,
    },
    /// Muestra estadísticas globales de la biblioteca
    Stats,
    /// Verifica la salud editorial de un libro (links rotos, etc.)
    Health {
        /// ID del libro en la base de datos
        book_id: i64,
    },
    /// Inicia el servidor OPDS embebido
    Serve {
        /// Puerto donde escuchar
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Borra un libro de la biblioteca
    Delete {
        /// ID del libro en la base de datos
        book_id: i64,
        /// Solo borrar de la base de datos (conservar archivo en disco)
        #[arg(short, long)]
        db_only: bool,
    },
    /// Lista todos los autores con cantidad de libros
    Authors {
        /// Muestra un autor por línea (formato vertical detallado)
        #[arg(short, long)]
        long: bool,
    },
    /// Exporta aliases y config a un archivo TOML
    ExportConfig {
        /// Ruta del archivo de salida
        path: PathBuf,
    },
    /// Importa aliases y config desde un archivo TOML
    ImportConfig {
        /// Ruta del archivo de entrada
        path: PathBuf,
    },
    /// Gestiona aliases del CLI (sin argumentos lista todos)
    Alias {
        /// Nombre del alias
        name: Option<String>,
        /// Comando al que apunta (sin argumentos borra el alias)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Borra un alias
    Unalias {
        /// Nombre del alias
        name: String,
    },
    /// Sale del modo interactivo
    Exit,
}

struct RubricaCompleter;

impl rustyline::Helper for RubricaCompleter {}
impl rustyline::highlight::Highlighter for RubricaCompleter {}
impl rustyline::hint::Hinter for RubricaCompleter {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}
impl rustyline::validate::Validator for RubricaCompleter {}

impl Completer for RubricaCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let line_up_to_pos = &line[..pos];
        let mut start = line_up_to_pos
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let mut token = &line_up_to_pos[start..];

        let tokens_before: Vec<&str> = line_up_to_pos[..start].split_whitespace().collect();

        let mut pairs = Vec::new();

        if tokens_before.is_empty() {
            // Primer token -> sugerir comandos
            for cmd in &["init", "import", "import-dir", "books", "authors", "stats", "health", "normalize", "serve", "delete", "export-config", "import-config", "alias", "unalias", "exit", "help"] {
                if cmd.starts_with(token) {
                    pairs.push(Pair {
                        display: cmd.to_string(),
                        replacement: cmd.to_string(),
                    });
                }
            }
        } else {
            let cmd = tokens_before[0];
            if token.starts_with('-') {
                let flags: &[&str] = match cmd {
                    "normalize" => &["--base-dir"],
                    "serve" => &["--port"],
                    "books" => &["--long", "--extralong", "--normalized", "--author", "--series", "--fts", "--ids"],
                    "authors" => &["--long"],
                    _ => &[],
                };
                for f in flags {
                    if f.starts_with(token) {
                        pairs.push(Pair {
                            display: f.to_string(),
                            replacement: f.to_string(),
                        });
                    }
                }
            } else if cmd == "import" {
                // Para import, el token abarca todo desde después del primer espacio tras "import"
                if let Some(import_pos) = line_up_to_pos.find("import ") {
                    start = import_pos + 7; // len("import ")
                    token = &line_up_to_pos[start..];
                }
                pairs.extend(path_completions(token));
            } else if cmd == "normalize" && tokens_before.len() == 1 {
                // Podríamos sugerir IDs de libros, pero requiere DB access async -> lo dejamos para después
            }
        }

        Ok((start, pairs))
    }
}

fn path_completions(token: &str) -> Vec<Pair> {
    let (dir_part, file_prefix) = if let Some(last_slash) = token.rfind('/') {
        (&token[..=last_slash], &token[last_slash + 1..])
    } else {
        ("./", token)
    };

    let mut pairs = Vec::new();
    let read_dir = if dir_part.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            std::fs::read_dir(format!("{}{}", home, &dir_part[1..]))
        } else {
            return pairs;
        }
    } else {
        std::fs::read_dir(dir_part)
    };

    if let Ok(entries) = read_dir {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(file_prefix) {
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let replacement = format!("{}{}{}", dir_part, name, if is_dir { "/" } else { "" });
                let display = format!("{}{}", name, if is_dir { "/" } else { "" });
                pairs.push(Pair { display, replacement });
            }
        }
    }
    pairs
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_url = to_db_url(&cli.db);

    match cli.command {
        Some(cmd) => execute(&db_url, cmd).await,
        None => run_repl(&db_url).await,
    }
}

/// Parsea una línea del REPL. El comando `import` es especial:
/// todo lo que sigue después del primer espacio se toma como un único path.
fn parse_repl_line(line: &str) -> anyhow::Result<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    // Si empieza con "import " (o "import\t"), tomar todo después como path único.
    if let Some(rest) = trimmed.strip_prefix("import ") {
        let path = rest.trim_start();
        return Ok(vec!["import".to_string(), path.to_string()]);
    }

    // Para cualquier otro comando, usar shell-words (respeta comillas y escapes).
    shell_words::split(trimmed).map_err(|e| anyhow::anyhow!("{}", e))
}

async fn run_repl(db_url: &str) -> Result<()> {
    println!("Rúbrica REPL - Modo interactivo");
    println!("Base de datos: {}", db_url);
    println!("Escribí 'help' para ver los comandos disponibles.");
    println!("Presioná TAB para autocompletar.");
    println!();

    let db = LibraryDb::new(db_url).await?;

    let config = Config::builder().tab_stop(4).build();
    let completer = RubricaCompleter;
    let mut rl: Editor<RubricaCompleter, rustyline::history::FileHistory> =
        Editor::with_config(config)?;
    rl.set_helper(Some(completer));

    let _ = rl.load_history(".rubrica_history");

    loop {
        let readline = rl.readline("rubrica> ");
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                rl.add_history_entry(line)?;

                if line == "help" || line == "h" {
                    print_help();
                    continue;
                }

                let mut words = match parse_repl_line(line) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("Error parseando comando: {}", e);
                        continue;
                    }
                };

                // Resolver alias: si el primer token es un alias, reemplazarlo
                if let Some(first) = words.first() {
                    if let Some(resolved) = db.get_alias(first).await? {
                        // Reemplazar el primer token por el comando resuelto del alias
                        let resolved_parts = shell_words::split(&resolved)
                            .unwrap_or_else(|_| vec![resolved.clone()]);
                        words.splice(0..1, resolved_parts);
                    }
                }

                let args = std::iter::once("rubrica".to_string())
                    .chain(words.into_iter())
                    .collect::<Vec<_>>();

                #[derive(Parser)]
                struct ReplCommand {
                    #[command(subcommand)]
                    cmd: Commands,
                }

                let cmd = match ReplCommand::try_parse_from(&args) {
                    Ok(c) => c.cmd,
                    Err(e) => {
                        eprintln!("{}", e);
                        continue;
                    }
                };

                if let Commands::Exit = cmd {
                    println!("Chau!");
                    break;
                }

                if let Err(e) = execute(db_url, cmd).await {
                    eprintln!("Error: {}", e);
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("Chau!");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    let _ = rl.save_history(".rubrica_history");
    Ok(())
}

/// Convierte una query tipo Google (`+obligatorio -excluido "frase exacta"`)
/// a la sintaxis nativa de FTS5 de SQLite.
fn parse_fts_query(raw: &str) -> String {
    let mut tokens = Vec::new();
    let mut chars = raw.chars().peekable();

    while chars.peek().is_some() {
        // Consumir espacios
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        let mut token = String::new();
        let mut in_quotes = false;

        while let Some(c) = chars.next() {
            if c == '"' {
                token.push('"');
                in_quotes = !in_quotes;
                if !in_quotes {
                    break;
                }
                continue;
            }
            if !in_quotes && c.is_whitespace() {
                break;
            }
            token.push(c);
        }

        if token.is_empty() {
            continue;
        }

        // +palabra → palabra (FTS5 ya es AND por defecto, solo quitamos el +)
        if token.starts_with('+') && token.len() > 1 {
            token = token[1..].to_string();
        }
        // -palabra → NOT palabra
        else if token.starts_with('-') && token.len() > 1 {
            token = format!("NOT {}", &token[1..]);
        }

        tokens.push(token);
    }

    tokens.join(" ")
}

fn print_help() {
    println!("Comandos disponibles:");
    println!("  init                     Inicializa la base de datos");
    println!("  import <ruta.epub>       Importa un EPUB");
    println!("  import-dir <ruta>        Importa recursivamente todos los EPUBs de un directorio");
    println!("  books [filtros]          Lista libros");
    println!("      --ids <1,5,10>       Filtrar por IDs específicos");
    println!("      --author <nombre>    Filtrar por autor (parcial)");
    println!("      --series <nombre>    Filtrar por serie (parcial)");
    println!("      --fts <query>        Buscar por título/autor (FTS5)");
    println!("                         Sintaxis: palabra1 palabra2 (AND)");
    println!("                                   -palabra (NOT)");
    println!("                                   +palabra (obligatorio)");
    println!("                                   \"frase exacta\"");
    println!("      --normalized         Solo libros normalizados");
    println!("      --long               Detalle completo de cada libro");
    println!("      --extralong          Info avanzada (tamaño, capítulos del EPUB)");
    println!("  authors                  Lista todos los autores");
    println!("  stats                    Estadísticas globales");
    println!("  health <id>              Verifica salud editorial de un libro");
    println!("  normalize <id> [--base-dir <dir>]  Normaliza ubicación de un libro");
    println!("  serve [--port <n>]       Inicia servidor OPDS (bloqueante)");
    println!("  delete <id> [--db-only]  Borra un libro (--db-only conserva el archivo)");
    println!("  export-config <ruta>     Exporta aliases a un archivo TOML");
    println!("  import-config <ruta>     Importa aliases desde un archivo TOML");
    println!("  alias <nombre> [comando]  Crea/actualiza alias (sin comando lista todos)");
    println!("  unalias <nombre>          Borra un alias");
    println!("  exit                     Sale del REPL");
    println!("  help                     Muestra esta ayuda");
}

async fn execute(db_url: &str, cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Init => {
            let _db = LibraryDb::new(db_url).await?;
            println!("Base de datos inicializada.");
        }
        Commands::Import { path } => {
            let db = LibraryDb::new(db_url).await?;
            let path_str = path.to_string_lossy().to_string();
            println!("Importando: {}", path_str);
            Pipeline::import_file(&db, path_str).await?;
            println!("Importación exitosa.");
        }
        Commands::ImportDir { path } => {
            import_directory(db_url, path).await?;
        }
        Commands::Books { long, extralong, normalized, author, series, fts, ids } => {
            let db = LibraryDb::new(db_url).await?;
            let books = if let Some(query) = fts {
                let parsed = parse_fts_query(&query);
                db.search_fts(&parsed).await?
            } else {
                let norm_filter = if normalized { Some(true) } else { None };
                let author_ref = author.as_deref();
                let series_ref = series.as_deref();
                let ids_ref = ids.as_deref();
                db.list_books_filtered(norm_filter, author_ref, series_ref, ids_ref).await?
            };
            if books.is_empty() {
                println!("No se encontraron libros con los filtros aplicados.");
            } else if extralong {
                print_books_extralong(&books)?;
            } else if long {
                print_books_vertical(&books);
            } else {
                print_books_table(&books);
            }
        }
        Commands::Normalize { book_id, base_dir } => {
            let db = LibraryDb::new(db_url).await?;
            let base_dir_str = base_dir.to_string_lossy().to_string();
            let base_dir_expanded = expand_tilde(&base_dir_str);
            let base_path = PathBuf::from(base_dir_expanded);
            println!("Normalizando libro {} en {:?}...", book_id, base_path);
            Organizer::normalize_book(&db, book_id, &base_path).await?;
            println!("Normalización completada.");
        }
        Commands::Stats => {
            let db = LibraryDb::new(db_url).await?;
            let metrics = Analytics::get_global_metrics(&db).await?;
            println!("Libros totales: {}", metrics.total_books);
            println!("Tiempo total de lectura: {}s", metrics.total_reading_time_secs);
        }
        Commands::Health { book_id } => {
            let db = LibraryDb::new(db_url).await?;
            let row: (String,) = sqlx::query_as("SELECT current_path FROM books WHERE id = ?")
                .bind(book_id)
                .fetch_one(db.pool())
                .await?;
            println!("Analizando salud del libro {}...", book_id);
            let report = Analytics::validate_links(book_id, &row.0).await?;
            println!("Links rotos: {}", report.broken_links);
            println!("Anclajes huérfanos: {}", report.orphan_anchors);
            println!("Discrepancias CSS: {}", report.css_discrepancies);
        }
        Commands::Serve { port } => {
            let db = LibraryDb::new(db_url).await?;
            println!("Iniciando servidor OPDS en http://0.0.0.0:{}/opds", port);
            println!("Presiona Ctrl+C para detener.");
            SyncSubsystem::start_opds_server(db, port).await?;
            tokio::signal::ctrl_c().await?;
            println!("Servidor detenido.");
        }
        Commands::Delete { book_id, db_only } => {
            let db = LibraryDb::new(db_url).await?;
            let deleted_file = db.delete_book(book_id, !db_only).await?;
            if db_only {
                println!("Libro {} eliminado de la base de datos.", book_id);
            } else if let Some(path) = deleted_file {
                println!("Libro {} eliminado. Archivo borrado: {}", book_id, path);
            } else {
                println!("Libro {} eliminado de la base de datos (archivo no encontrado en disco).", book_id);
            }
        }
        Commands::Authors { long } => {
            let db = LibraryDb::new(db_url).await?;
            let authors = db.list_authors().await?;
            if authors.is_empty() {
                println!("No hay autores en la biblioteca.");
            } else if long {
                print_authors_vertical(&authors);
            } else {
                print_authors_table(&authors);
            }
        }
        Commands::ExportConfig { path } => {
            let db = LibraryDb::new(db_url).await?;
            let aliases = db.list_aliases().await?;
            let mut config = rubrica::RubricaConfig::default();
            for a in aliases {
                config.aliases.insert(a.name, a.command);
            }
            let toml = config.to_toml()?;
            tokio::fs::write(&path, toml).await?;
            println!("Config exportada a {}", path.display());
        }
        Commands::ImportConfig { path } => {
            let db = LibraryDb::new(db_url).await?;
            let contents = tokio::fs::read_to_string(&path).await?;
            let config = rubrica::RubricaConfig::from_toml(&contents)?;
            let mut count = 0;
            for (name, cmd) in config.aliases {
                db.set_alias(&name, &cmd).await?;
                count += 1;
            }
            println!("{} alias(es) importados desde {}", count, path.display());
        }
        Commands::Alias { name, command } => {
            let db = LibraryDb::new(db_url).await?;
            match name {
                Some(alias_name) => {
                    let cmd = command.join(" ");
                    if cmd.is_empty() {
                        // Sin comando -> listar alias
                        let aliases = db.list_aliases().await?;
                        if aliases.is_empty() {
                            println!("No hay aliases definidos.");
                        } else {
                            println!("Aliases:");
                            for a in aliases {
                                println!("  {} = {}", a.name, a.command);
                            }
                        }
                    } else {
                        db.set_alias(&alias_name, &cmd).await?;
                        println!("Alias '{}' = '{}' guardado.", alias_name, cmd);
                    }
                }
                None => {
                    // Sin nombre -> listar todos los aliases
                    let aliases = db.list_aliases().await?;
                    if aliases.is_empty() {
                        println!("No hay aliases definidos.");
                    } else {
                        println!("Aliases:");
                        for a in aliases {
                            println!("  {} = {}", a.name, a.command);
                        }
                    }
                }
            }
        }
        Commands::Unalias { name } => {
            let db = LibraryDb::new(db_url).await?;
            db.delete_alias(&name).await?;
            println!("Alias '{}' eliminado.", name);
        }
        Commands::Exit => {}
    }

    Ok(())
}

async fn import_directory(db_url: &str, path: PathBuf) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();

    // 1. Encontrar todos los .epub recursivamente
    let mut epub_paths = Vec::new();
    for entry in walkdir::WalkDir::new(&path) {
        if let Ok(e) = entry {
            if e.file_type().is_file() {
                if let Some(ext) = e.path().extension() {
                    if ext.eq_ignore_ascii_case("epub") {
                        epub_paths.push(e.path().to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    if epub_paths.is_empty() {
        println!("No se encontraron archivos .epub en {}", path_str);
        return Ok(());
    }

    println!("Encontrados {} EPUB(s) en {}. Importando...", epub_paths.len(), path_str);

    // 2. Procesar en batch
    let db = LibraryDb::new(db_url).await?;
    let total = epub_paths.len();
    let mut rx = Pipeline::import_batch(db, epub_paths);

    let mut imported = 0;
    let mut duplicates = 0;
    let mut errors = 0;

    while let Some((path, result)) = rx.recv().await {
        let n = imported + duplicates + errors + 1;
        match result {
            Ok(ImportStatus::Imported) => {
                imported += 1;
                println!("  [{}/{}] OK: {}", n, total, path);
            }
            Ok(ImportStatus::Duplicate { existing_id }) => {
                duplicates += 1;
                println!("  [{}/{}] DUPLICADO (ID: {}): {}", n, total, existing_id, path);
            }
            Err(e) => {
                errors += 1;
                eprintln!("  [{}/{}] ERROR: {} - {}", n, total, path, e);
            }
        }
    }

    println!();
    println!("Resumen:");
    println!("  Importados: {}", imported);
    println!("  Duplicados: {}", duplicates);
    println!("  Errores:    {}", errors);

    Ok(())
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}

fn to_db_url(db: &str) -> String {
    if db.starts_with("sqlite:") {
        return db.to_string();
    }
    let path = PathBuf::from(db);
    let abs = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    format!("sqlite://{}", abs.display())
}

// ─── Funciones de impresión responsive ─────────────────────────────────────

fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}

/// Imprime libros en formato tabla horizontal con columnas proporcionales al ancho de terminal.
fn print_books_table(books: &[rubrica::library_db::BookListItem]) {
    let tw = term_width();
    // Columnas fijas: ID(5) + sep(1) + Normalizado(13) = 19
    let fixed = 5 + 1 + 13;
    let available = tw.saturating_sub(fixed).max(40);

    let title_w = (available as f32 * 0.50) as usize;
    let author_w = (available as f32 * 0.28) as usize;
    let series_w = available.saturating_sub(title_w + author_w);

    // Mínimos para que no se aplasten
    let title_w = title_w.max(15);
    let author_w = author_w.max(10);
    let series_w = series_w.max(8);

    println!(
        "{:>5} {:<title$} {:<author$} {:<series$} {}",
        "ID", "Título", "Autor", "Serie", "Normalizado",
        title = title_w, author = author_w, series = series_w
    );

    for b in books {
        let series = b.series_name.as_deref().unwrap_or("-");
        let norm = if b.is_normalized { "Sí" } else { "No" };
        println!(
            "{:>5} {:<title$} {:<author$} {:<series$} {}",
            b.id,
            truncate(&b.title, title_w),
            truncate(&b.author_name, author_w),
            truncate(series, series_w),
            norm,
            title = title_w, author = author_w, series = series_w
        );
    }
}

/// Imprime libros en formato vertical detallado (un libro por bloque).
fn print_books_vertical(books: &[rubrica::library_db::BookListItem]) {
    for b in books {
        let series = b.series_name.as_deref().unwrap_or("-");
        let norm = if b.is_normalized { "Sí" } else { "No" };
        let hash = b.file_hash.as_deref().unwrap_or("-");
        let date = b.date_added.format("%Y-%m-%d %H:%M").to_string();
        println!("ID:          {}", b.id);
        println!("Título:      {}", b.title);
        println!("Autor:       {}", b.author_name);
        println!("Serie:       {}", series);
        println!("Normalizado: {}", norm);
        println!("Fecha:       {}", date);
        println!("Hash:        {}", hash);
        println!("Ruta:        {}", b.current_path);
        println!();
    }
}

/// Extrae tamaño en bytes y lista de capítulos de un archivo EPUB.
fn extract_epub_info(epub_path: &str) -> Result<(u64, Vec<String>)> {
    let path = epub_path.to_string();
    let (size, chapters) = std::thread::spawn(move || -> anyhow::Result<(u64, Vec<String>)> {
        let core = gutencore::GutenCore::open_epub(&path)
            .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;

        let size = std::fs::metadata(&path)?.len();
        let toc = core.get_toc()
            .map_err(|e| anyhow::anyhow!("TOC error: {}", e))?;
        let chapters: Vec<String> = toc.into_iter().map(|e| e.title).collect();

        Ok((size, chapters))
    })
    .join()
    .unwrap()?;

    Ok((size, chapters))
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.1} {}", size, UNITS[unit_idx])
}

/// Imprime libros con información avanzada del EPUB (tamaño, capítulos).
fn print_books_extralong(books: &[rubrica::library_db::BookListItem]) -> Result<()> {
    for b in books {
        let series = b.series_name.as_deref().unwrap_or("-");
        let norm = if b.is_normalized { "Sí" } else { "No" };
        let hash = b.file_hash.as_deref().unwrap_or("-");
        let date = b.date_added.format("%Y-%m-%d %H:%M").to_string();
        let (size, chapters) = extract_epub_info(&b.current_path).unwrap_or((0, Vec::new()));
        let size_str = if size > 0 { format_size(size) } else { "-".to_string() };

        println!("ID:          {}", b.id);
        println!("Título:      {}", b.title);
        println!("Autor:       {}", b.author_name);
        println!("Serie:       {}", series);
        println!("Normalizado: {}", norm);
        println!("Fecha:       {}", date);
        println!("Hash:        {}", hash);
        println!("Tamaño:      {}", size_str);
        println!("Ruta:        {}", b.current_path);

        if !chapters.is_empty() {
            println!("Capítulos:");
            for (i, ch) in chapters.iter().enumerate() {
                println!("  {}. {}", i + 1, ch);
            }
        } else {
            println!("Capítulos:   (no disponibles)");
        }
        println!();
    }
    Ok(())
}

/// Imprime autores en formato tabla horizontal proporcional.
fn print_authors_table(authors: &[rubrica::library_db::AuthorStats]) {
    let tw = term_width();
    let fixed = 5 + 1 + 7; // ID + sep + Libros
    let available = tw.saturating_sub(fixed).max(20);
    let name_w = available.max(10);

    println!("{:>5} {:>6} {:<name$}", "ID", "Libros", "Autor", name = name_w);
    for a in authors {
        println!(
            "{:>5} {:>6} {:<name$}",
            a.id, a.book_count, truncate(&a.name, name_w), name = name_w
        );
    }
}

/// Imprime autores en formato vertical detallado.
fn print_authors_vertical(authors: &[rubrica::library_db::AuthorStats]) {
    for a in authors {
        println!("ID:     {}", a.id);
        println!("Autor:  {}", a.name);
        println!("Libros: {}", a.book_count);
        println!();
    }
}
