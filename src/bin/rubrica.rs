use anyhow::Result;
use clap::{Parser, Subcommand};
use rubrica::{Analytics, LibraryDb, Organizer, Pipeline, SyncSubsystem};
use rubrica::pipeline::ImportStatus;
use rustyline::completion::{Completer, Pair};
use rustyline::{Config, Editor};
use colored::*;
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
        /// Limitar cantidad de resultados (0 = sin límite)
        #[arg(long, default_value_t = 0)]
        limit: usize,
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
        /// También borrar el archivo en disco (por defecto solo se borra de la base de datos)
        #[arg(short, long)]
        with_file: bool,
    },
    /// Lista todos los autores con cantidad de libros
    Authors {
        /// Muestra un autor por línea (formato vertical detallado)
        #[arg(short, long)]
        long: bool,
        /// Limitar cantidad de resultados (0 = sin límite)
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Lista todas las series con cantidad de libros
    Series {
        /// Muestra una serie por línea (formato vertical detallado)
        #[arg(short, long)]
        long: bool,
        /// Limitar cantidad de resultados (0 = sin límite)
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Crea una nueva serie vacía
    AddSeries {
        /// Nombre de la serie
        name: String,
    },
    /// Borra una serie
    DeleteSeries {
        /// ID de la serie en la base de datos
        series_id: i64,
        /// Desvincular libros asociados antes de borrar
        #[arg(short, long)]
        force: bool,
    },
    /// Asigna un libro a una serie
    AssignSeries {
        /// ID del libro en la base de datos
        book_id: i64,
        /// ID de la serie en la base de datos
        series_id: i64,
    },
    /// Desvincula un libro de su serie
    UnassignSeries {
        /// ID del libro en la base de datos
        book_id: i64,
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
            for cmd in &["init", "import", "import-dir", "books", "authors", "series", "add-series", "delete-series", "assign-series", "unassign-series", "stats", "health", "normalize", "serve", "delete", "export-config", "import-config", "alias", "unalias", "exit", "help"] {
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
                    "books" => &["--long", "--extralong", "--normalized", "--author", "--series", "--fts", "--ids", "--limit"],
                    "authors" | "series" => &["--long", "--limit"],
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
    println!("{}", "═══════════════════════════════════════════".cyan().dimmed());
    println!("  {} {}", "Rúbrica".cyan().bold(), "REPL - Modo interactivo".cyan());
    println!("{}", "═══════════════════════════════════════════".cyan().dimmed());
    println!("  {} {}", "Base de datos:".dimmed(), db_url.dimmed());
    println!("  {} {}", "Tip:".dimmed(), "escribí 'help' para ver los comandos disponibles.".dimmed());
    println!("  {} {}", "Tip:".dimmed(), "presioná TAB para autocompletar.".dimmed());
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
                        eprintln!("{} {}", "Error parseando comando:".red(), e);
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
                        eprintln!("{}", e.to_string().red());
                        continue;
                    }
                };

                if let Commands::Exit = cmd {
                    println!("{}", "Chau!".green());
                    break;
                }

                if let Err(e) = execute(db_url, cmd).await {
                    eprintln!("{} {}", "Error:".red(), e.to_string().red());
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("{}", "^C".dimmed());
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("{}", "Chau!".green());
                break;
            }
            Err(err) => {
                eprintln!("{} {:?}", "Error:".red(), err);
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
    println!("{}", "Comandos disponibles:".cyan().bold());
    println!("  {}   Inicializa la base de datos", "init".yellow());
    println!("  {}   Importa un EPUB", "import".yellow());
    println!("  {}   Importa recursivamente todos los EPUBs de un directorio", "import-dir".yellow());
    println!("  {}   Lista libros", "books".yellow());
    println!("      {}       Filtrar por IDs específicos", "--ids".dimmed());
    println!("      {}    Filtrar por autor (parcial)", "--author".dimmed());
    println!("      {}    Filtrar por serie (parcial)", "--series".dimmed());
    println!("      {}     Buscar por título/autor (FTS5)", "--fts".dimmed());
    println!("                         {} palabra1 palabra2 {}", "Sintaxis:".dimmed(), "(AND)".dimmed());
    println!("                                   {} -palabra {}", "".dimmed(), "(NOT)".dimmed());
    println!("                                   {} +palabra {}", "".dimmed(), "(obligatorio)".dimmed());
    println!("                                   {} \"frase exacta\"", "".dimmed());
    println!("      {}       Solo libros normalizados", "--normalized".dimmed());
    println!("      {}        Detalle completo de cada libro", "--long".dimmed());
    println!("      {}     Info avanzada (tamaño, capítulos del EPUB)", "--extralong".dimmed());
    println!("      {}       Limitar cantidad de resultados", "--limit".dimmed());
    println!("  {}   Lista todos los autores", "authors".yellow());
    println!("  {}   Lista todas las series", "series".yellow());
    println!("  {}   Crea una nueva serie vacía", "add-series".yellow());
    println!("  {}   Borra una serie (usar --force si tiene libros)", "delete-series".yellow());
    println!("  {}   Asigna un libro a una serie", "assign-series".yellow());
    println!("  {}   Desvincula un libro de su serie", "unassign-series".yellow());
    println!("  {}   Estadísticas globales", "stats".yellow());
    println!("  {}   Verifica salud editorial de un libro", "health".yellow());
    println!("  {}   Normaliza ubicación de un libro", "normalize".yellow());
    println!("  {}   Inicia servidor OPDS (bloqueante)", "serve".yellow());
    println!("  {}   Borra de la base de datos", "delete".yellow());
    println!("  {}   Exporta aliases a un archivo TOML", "export-config".yellow());
    println!("  {}   Importa aliases desde un archivo TOML", "import-config".yellow());
    println!("  {}   Crea/actualiza alias", "alias".yellow());
    println!("  {}   Borra un alias", "unalias".yellow());
    println!("  {}   Sale del REPL", "exit".yellow());
    println!("  {}   Muestra esta ayuda", "help".yellow());
}

async fn execute(db_url: &str, cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Init => {
            let _db = LibraryDb::new(db_url).await?;
            println!("{}", "Base de datos inicializada.".green());
        }
        Commands::Import { path } => {
            let db = LibraryDb::new(db_url).await?;
            let path_str = path.to_string_lossy().to_string();
            println!("{} {}", "Importando:".cyan(), path_str.dimmed());
            Pipeline::import_file(&db, path_str).await?;
            println!("{}", "Importación exitosa.".green());
        }
        Commands::ImportDir { path } => {
            import_directory(db_url, path).await?;
        }
        Commands::Books { long, extralong, normalized, author, series, fts, ids, limit } => {
            let db = LibraryDb::new(db_url).await?;
            let mut books = if let Some(query) = fts {
                let parsed = parse_fts_query(&query);
                db.search_fts(&parsed).await?
            } else {
                let norm_filter = if normalized { Some(true) } else { None };
                let author_ref = author.as_deref();
                let series_ref = series.as_deref();
                let ids_ref = ids.as_deref();
                db.list_books_filtered(norm_filter, author_ref, series_ref, ids_ref).await?
            };
            let total = books.len();
            if limit > 0 && books.len() > limit {
                books.truncate(limit);
            }
            if books.is_empty() {
                println!("{}", "No se encontraron libros con los filtros aplicados.".yellow());
            } else if extralong {
                print_books_extralong(&books).await?;
            } else if long {
                print_books_vertical(&books);
            } else {
                print_books_table(&books);
            }
            if limit > 0 && total > limit {
                println!("{} {} {}", "Mostrando".dimmed(), limit.to_string().yellow(), format!("de {} registros (usá --limit 0 para ver todos)", total).dimmed());
            }
        }
        Commands::Normalize { book_id, base_dir } => {
            let db = LibraryDb::new(db_url).await?;
            let base_dir_str = base_dir.to_string_lossy().to_string();
            let base_dir_expanded = expand_tilde(&base_dir_str);
            let base_path = PathBuf::from(base_dir_expanded);
            println!("{} {} en {}...", "Normalizando libro".cyan(), book_id, base_path.display().to_string().dimmed());
            Organizer::normalize_book(&db, book_id, &base_path).await?;
            println!("{}", "Normalización completada.".green());
        }
        Commands::Stats => {
            let db = LibraryDb::new(db_url).await?;
            let metrics = Analytics::get_global_metrics(&db).await?;
            println!("{} {}", "Libros totales:".cyan().bold(), metrics.total_books);
            println!("{} {}s", "Tiempo total de lectura:".cyan().bold(), metrics.total_reading_time_secs);
        }
        Commands::Health { book_id } => {
            let db = LibraryDb::new(db_url).await?;
            let row: (String,) = sqlx::query_as("SELECT current_path FROM books WHERE id = ?")
                .bind(book_id)
                .fetch_one(db.pool())
                .await?;
            println!("{} {}", "Analizando salud del libro".cyan(), book_id);
            let report = Analytics::validate_links(book_id, &row.0).await?;
            println!("{} {}", "Links rotos:".cyan(), if report.broken_links > 0 { report.broken_links.to_string().red() } else { "0".green() });
            println!("{} {}", "Anclajes huérfanos:".cyan(), report.orphan_anchors);
            println!("{} {}", "Discrepancias CSS:".cyan(), report.css_discrepancies);
        }
        Commands::Serve { port } => {
            let db = LibraryDb::new(db_url).await?;
            println!("{} http://0.0.0.0:{}/opds", "Iniciando servidor OPDS".cyan().bold(), port);
            println!("{}", "Presiona Ctrl+C para detener.".dimmed());
            SyncSubsystem::start_opds_server(db, port).await?;
            tokio::signal::ctrl_c().await?;
            println!("{}", "Servidor detenido.".green());
        }
        Commands::Delete { book_id, with_file } => {
            let db = LibraryDb::new(db_url).await?;
            let deleted_file = db.delete_book(book_id, with_file).await?;
            if with_file {
                if let Some(path) = deleted_file {
                    println!("{} {} {}", "Libro".red(), book_id, "eliminado. Archivo borrado:".red());
                    println!("  {}", path.dimmed());
                } else {
                    println!("{} {}", "Libro".red(), "eliminado de la base de datos (archivo no encontrado en disco).".yellow());
                }
            } else {
                println!("{} {}", "Libro".yellow(), "eliminado de la base de datos.".yellow());
            }
        }
        Commands::Authors { long, limit } => {
            let db = LibraryDb::new(db_url).await?;
            let mut authors = db.list_authors().await?;
            let total = authors.len();
            if limit > 0 && authors.len() > limit {
                authors.truncate(limit);
            }
            if authors.is_empty() {
                println!("{}", "No hay autores en la biblioteca.".yellow());
            } else if long {
                print_authors_vertical(&authors);
            } else {
                print_authors_table(&authors);
            }
            if limit > 0 && total > limit {
                println!("{} {} {}", "Mostrando".dimmed(), limit.to_string().yellow(), format!("de {} registros (usá --limit 0 para ver todos)", total).dimmed());
            }
        }
        Commands::Series { long, limit } => {
            let db = LibraryDb::new(db_url).await?;
            let mut series_list = db.list_series().await?;
            let total = series_list.len();
            if limit > 0 && series_list.len() > limit {
                series_list.truncate(limit);
            }
            if series_list.is_empty() {
                println!("{}", "No hay series en la biblioteca.".yellow());
            } else if long {
                print_series_vertical(&series_list);
            } else {
                print_series_table(&series_list);
            }
            if limit > 0 && total > limit {
                println!("{} {} {}", "Mostrando".dimmed(), limit.to_string().yellow(), format!("de {} registros (usá --limit 0 para ver todos)", total).dimmed());
            }
        }
        Commands::AddSeries { name } => {
            let db = LibraryDb::new(db_url).await?;
            let id = db.add_series(&name).await?;
            println!("{} {} {} {}", "Serie".green(), name.yellow(), "creada con ID".green(), id.to_string().cyan());
        }
        Commands::DeleteSeries { series_id, force } => {
            let db = LibraryDb::new(db_url).await?;
            db.delete_series(series_id, force).await?;
            println!("{} {}", "Serie".green(), "eliminada.".green());
        }
        Commands::AssignSeries { book_id, series_id } => {
            let db = LibraryDb::new(db_url).await?;
            db.assign_book_series(book_id, series_id).await?;
            println!("{} {} {} {} {}", "Libro".green(), book_id.to_string().cyan(), "asignado a serie".green(), series_id.to_string().cyan(), "✓".green());
        }
        Commands::UnassignSeries { book_id } => {
            let db = LibraryDb::new(db_url).await?;
            db.unassign_book_series(book_id).await?;
            println!("{} {} {}", "Libro".green(), book_id.to_string().cyan(), "desvinculado de serie.".green());
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
            println!("{} {}", "Config exportada a".green(), path.display().to_string().dimmed());
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
            println!("{} {} {} {}", count.to_string().green(), "alias(es) importados desde".green(), path.display().to_string().dimmed(), "✓".green());
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
                            println!("{}", "No hay aliases definidos.".yellow());
                        } else {
                            println!("{}", "Aliases:".cyan().bold());
                            for a in aliases {
                                println!("  {} = {}", a.name.yellow(), a.command.dimmed());
                            }
                        }
                    } else {
                        db.set_alias(&alias_name, &cmd).await?;
                        println!("{} {} {} {} {}", "Alias".green(), alias_name.yellow(), "=".dimmed(), cmd.dimmed(), "guardado.".green());
                    }
                }
                None => {
                    // Sin nombre -> listar todos los aliases
                    let aliases = db.list_aliases().await?;
                    if aliases.is_empty() {
                        println!("{}", "No hay aliases definidos.".yellow());
                    } else {
                        println!("{}", "Aliases:".cyan().bold());
                        for a in aliases {
                            println!("  {} = {}", a.name.yellow(), a.command.dimmed());
                        }
                    }
                }
            }
        }
        Commands::Unalias { name } => {
            let db = LibraryDb::new(db_url).await?;
            db.delete_alias(&name).await?;
            println!("{} {} {}", "Alias".green(), name.yellow(), "eliminado.".green());
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
        println!("{} {}", "No se encontraron archivos .epub en".yellow(), path_str.dimmed());
        return Ok(());
    }

    println!("{} {} {} {}", "Encontrados".cyan(), epub_paths.len().to_string().cyan().bold(), "EPUB(s) en".cyan(), path_str.dimmed());

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
                println!("  [{}/{}] {} {}", n, total, "OK".green(), path.dimmed());
            }
            Ok(ImportStatus::Duplicate { existing_id }) => {
                duplicates += 1;
                println!("  [{}/{}] {} (ID: {}) {}", n, total, "DUPLICADO".yellow(), existing_id, path.dimmed());
            }
            Err(e) => {
                errors += 1;
                eprintln!("  [{}/{}] {} {} - {}", n, total, "ERROR".red(), path.dimmed(), e.to_string().red());
            }
        }
    }

    println!();
    println!("{}", "Resumen:".cyan().bold());
    println!("  {} {}", "Importados:".green(), imported.to_string().green().bold());
    println!("  {} {}", "Duplicados:".yellow(), duplicates.to_string().yellow());
    println!("  {} {}", "Errores:".red(), errors.to_string().red().bold());

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
        "{} {:<title$} {:<author$} {:<series$} {}",
        "ID".cyan().bold(),
        "Título".cyan().bold(),
        "Autor".cyan().bold(),
        "Serie".cyan().bold(),
        "Normalizado".cyan().bold(),
        title = title_w, author = author_w, series = series_w
    );

    for b in books {
        let series = b.series_name.as_deref().unwrap_or("-");
        let norm = if b.is_normalized { "Sí".green() } else { "No".red() };
        println!(
            "{} {:<title$} {:<author$} {:<series$} {}",
            format!("{:>5}", b.id).cyan(),
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
        let norm = if b.is_normalized { "Sí".green() } else { "No".red() };
        let hash = b.file_hash.as_deref().unwrap_or("-");
        let date = b.date_added.format("%Y-%m-%d %H:%M").to_string();
        println!("{} {}", "ID:".cyan().bold(), b.id.to_string().cyan());
        println!("{} {}", "Título:".cyan().bold(), b.title);
        println!("{} {}", "Autor:".cyan().bold(), b.author_name);
        println!("{} {}", "Serie:".cyan().bold(), series);
        println!("{} {}", "Normalizado:".cyan().bold(), norm);
        println!("{} {}", "Fecha:".cyan().bold(), date);
        println!("{} {}", "Hash:".cyan().bold(), hash.dimmed());
        println!("{} {}", "Ruta:".cyan().bold(), b.current_path.dimmed());
        println!();
    }
}

/// Extrae la lista de capítulos de un archivo EPUB (best-effort).
async fn extract_chapters(epub_path: &str) -> Result<Vec<String>> {
    let path = epub_path.to_string();
    tokio::task::spawn_blocking(move || {
        let core = gutencore::GutenCore::open_epub(&path)
            .map_err(|e| anyhow::anyhow!("GutenCore error: {}", e))?;

        let toc = core.get_toc()
            .map_err(|e| anyhow::anyhow!("TOC error: {}", e))?;
        let chapters: Vec<String> = toc.into_iter().map(|e| e.title).collect();

        Ok(chapters)
    })
    .await?
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
async fn print_books_extralong(books: &[rubrica::library_db::BookListItem]) -> Result<()> {
    for b in books {
        let series = b.series_name.as_deref().unwrap_or("-");
        let norm = if b.is_normalized { "Sí" } else { "No" };
        let hash = b.file_hash.as_deref().unwrap_or("-");
        let date = b.date_added.format("%Y-%m-%d %H:%M").to_string();

        // Tamaño: siempre disponible
        let size = tokio::fs::metadata(&b.current_path).await?.len();
        let size_str = format_size(size);

        // Capítulos: best-effort (pueden fallar por DTD en toc.ncx)
        let chapters = match extract_chapters(&b.current_path).await {
            Ok(ch) => ch,
            Err(e) => {
                eprintln!("  [AVISO] No se pudieron leer capítulos para ID {}: {}", b.id, e);
                Vec::new()
            }
        };

        println!("{} {}", "ID:".cyan().bold(), b.id.to_string().cyan());
        println!("{} {}", "Título:".cyan().bold(), b.title);
        println!("{} {}", "Autor:".cyan().bold(), b.author_name);
        println!("{} {}", "Serie:".cyan().bold(), series);
        println!("{} {}", "Normalizado:".cyan().bold(), norm);
        println!("{} {}", "Fecha:".cyan().bold(), date);
        println!("{} {}", "Hash:".cyan().bold(), hash.dimmed());
        println!("{} {}", "Tamaño:".cyan().bold(), size_str.green());
        println!("{} {}", "Ruta:".cyan().bold(), b.current_path.dimmed());

        if !chapters.is_empty() {
            println!("{}", "Capítulos:".cyan().bold());
            for (i, ch) in chapters.iter().enumerate() {
                println!("  {} {}", format!("{}.", i + 1).dimmed(), ch);
            }
        } else {
            println!("{} {}", "Capítulos:".cyan().bold(), "(no disponibles)".red());
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

    println!(
        "{} {} {:<name$}",
        "ID".cyan().bold(),
        "Libros".cyan().bold(),
        "Autor".cyan().bold(),
        name = name_w
    );
    for a in authors {
        println!(
            "{} {} {:<name$}",
            format!("{:>5}", a.id).cyan(),
            format!("{:>6}", a.book_count).green(),
            truncate(&a.name, name_w),
            name = name_w
        );
    }
}

/// Imprime autores en formato vertical detallado.
fn print_authors_vertical(authors: &[rubrica::library_db::AuthorStats]) {
    for a in authors {
        println!("{} {}", "ID:".cyan().bold(), a.id.to_string().cyan());
        println!("{} {}", "Autor:".cyan().bold(), a.name);
        println!("{} {}", "Libros:".cyan().bold(), a.book_count.to_string().green());
        println!();
    }
}

/// Imprime series en formato tabla horizontal proporcional.
fn print_series_table(series_list: &[rubrica::library_db::SeriesStats]) {
    let tw = term_width();
    let fixed = 5 + 1 + 7; // ID + sep + Libros
    let available = tw.saturating_sub(fixed).max(20);
    let name_w = available.max(10);

    println!(
        "{} {} {:<name$}",
        "ID".cyan().bold(),
        "Libros".cyan().bold(),
        "Serie".cyan().bold(),
        name = name_w
    );
    for s in series_list {
        println!(
            "{} {} {:<name$}",
            format!("{:>5}", s.id).cyan(),
            format!("{:>6}", s.book_count).green(),
            truncate(&s.name, name_w),
            name = name_w
        );
    }
}

/// Imprime series en formato vertical detallado.
fn print_series_vertical(series_list: &[rubrica::library_db::SeriesStats]) {
    for s in series_list {
        println!("{} {}", "ID:".cyan().bold(), s.id.to_string().cyan());
        println!("{} {}", "Serie:".cyan().bold(), s.name);
        println!("{} {}", "Libros:".cyan().bold(), s.book_count.to_string().green());
        println!();
    }
}
