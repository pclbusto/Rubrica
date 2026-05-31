# Rúbrica

> **Rúbrica** es un administrador de bibliotecas EPUB de alto rendimiento, escrito en Rust. Diseñado como una librería agnóstica de UI con un CLI/REPL potente, permite importar, organizar, buscar y servir colecciones de libros electrónicos con eficiencia y elegancia.

---

## Características principales

- **Importación robusta de EPUBs** con validación estructural mediante [`gutencore`](https://github.com/pclbusto/GutenAIR).
- **Base de datos SQLite** con índice FTS5 para búsquedas rápidas en metadatos.
- **Deduplicación automática** por hash SHA-256.
- **Organización en disco** con esquema de carpetas normalizado (`Autor/Serie/Título.epub`).
- **Servidor OPDS embebido** para sincronización con lectores compatibles.
- **CLI completo y REPL interactivo** con autocompletado, historial y aliases persistentes.
- **Formatos de salida flexibles**: tabla horizontal, detalle vertical y modo avanzado con análisis del EPUB.

---

## Arquitectura

Rúbrica está construido como una **librería pura** (`lib.rs`) con un binario CLI (`src/bin/rubrica.rs`). Cada subsistema es independiente y puede usarse programáticamente.

| Módulo | Responsabilidad |
|---|---|
| `library_db` | Esquema SQLite, migraciones, queries, aliases CRUD, índice FTS5 |
| `gutencore` | Adaptador para `gutencore` (GutenAIR): descompresión ZIP, parseo OPF, extracción de metadatos |
| `pipeline` | Importación de archivos con validación, deduplicación e indexación FTS5 |
| `organizer` | Normalización de rutas en disco, validación post-copia |
| `analytics` | Métricas de biblioteca y validación de salud editorial (links rotos, etc.) |
| `sync` | Servidor OPDS embebido con Axum (feed dinámico + descargas) |
| `config` | Serialización TOML de configuración (aliases exportables) |

---

## Instalación

### Requisitos

- Rust 1.78+ (edition 2024)
- SQLite 3.35+ (para FTS5)

### Compilación

```bash
git clone https://github.com/pclbusto/Rubrica.git
cd Rubrica
cargo build --release
```

El binario se generará en `target/release/rubrica`.

---

## Uso rápido

### Inicializar la biblioteca

```bash
./rubrica init
```

Crea `library.db` con el esquema completo, índice FTS5 y tablas de aliases.

### Importar libros

```bash
# Un solo EPUB
./rubrica import ~/Downloads/mi-libro.epub

# Todo un directorio recursivamente
./rubrica import-dir ~/Descargas/Libros/
```

### Listar y buscar

```bash
# Tabla compacta de todos los libros
./rubrica books

# Buscar por título o autor (FTS5)
./rubrica books --fts "Dostoievski"

# Filtrar por autor
./rubrica books --author "George R. R. Martin"

# Filtrar por IDs específicos
./rubrica books --ids 42
./rubrica books --ids 1,5,10

# Ver detalle completo
./rubrica books --ids 42 --long

# Ver información avanzada del EPUB (tamaño, capítulos)
./rubrica books --ids 42 --extralong
```

### Organizar archivos en disco

```bash
./rubrica normalize 42
```

Mueve el archivo a una estructura limpia: `~/Books/Rubrica/Autor/Serie/Título.epub`.

### Servidor OPDS

```bash
./rubrica serve --port 8080
```

Expone un catálogo OPDS en `http://localhost:8080/opds`, compatible con lectores como FBReader, Moon+ Reader, etc.

### Aliases persistentes

```bash
# Crear alias
./rubrica alias l "books --long"

# Usar alias (también en el REPL)
./rubrica alias run l

# Exportar/importar aliases
./rubrica export-config aliases.toml
./rubrica import-config aliases.toml
```

### REPL interactivo

```bash
./rubrica
```

Inicia el modo interactivo con:
- **Autocompletado** de comandos y flags (`Tab`)
- **Historial** persistente (`.rubrica_history`)
- **Aliases** resueltos automáticamente

```
> books --fts "Dragon"
> books --ids 42 --extralong
> health 42
> serve --port 3000
> exit
```

---

## Comandos del CLI

| Comando | Descripción |
|---|---|
| `init` | Inicializa la base de datos |
| `import <ruta.epub>` | Importa un EPUB |
| `import-dir <ruta>` | Importa recursivamente todos los EPUBs |
| `books [filtros]` | Lista libros con múltiples filtros |
| `authors` | Lista autores con cantidad de libros |
| `stats` | Estadísticas globales de la biblioteca |
| `health <id>` | Valida salud editorial (links rotos, CSS, anclajes) |
| `normalize <id>` | Reubica el archivo en estructura normalizada |
| `serve [--port]` | Inicia servidor OPDS embebido |
| `delete <id> [--db-only]` | Elimina un libro (con o sin archivo) |
| `export-config <ruta>` | Exporta aliases a TOML |
| `import-config <ruta>` | Importa aliases desde TOML |
| `alias <nombre> [comando]` | Crea, actualiza o lista aliases |
| `unalias <nombre>` | Elimina un alias |

### Filtros de `books`

| Flag | Descripción |
|---|---|
| `--ids <1,5,10>` | Filtrar por IDs específicos |
| `--author <nombre>` | Filtrar por autor (parcial, LIKE) |
| `--series <nombre>` | Filtrar por serie (parcial, LIKE) |
| `--fts <query>` | Búsqueda FTS5 en título y autor |
| `--normalized` | Solo libros con ubicación normalizada |
| `--long` | Formato vertical detallado |
| `--extralong` | Info avanzada: tamaño del archivo + índice de capítulos del EPUB |

### Sintaxis de búsqueda FTS5 (`--fts`)

Rúbrica acepta una sintaxis tipo Google que traduce internamente a FTS5 de SQLite:

| Querés buscar... | Escribís... |
|---|---|
| Ambas palabras (AND) | `Dragon Dance` |
| Obligatorio | `+Dostoievski Crimen` |
| Excluir | `Dragon -Dance` |
| Frase exacta | `"The Name of the Wind"` |

---

## Formato de salida

### Tabla horizontal (por defecto)

Ajusta automáticamente columnas al ancho de la terminal:

```
   ID Título              Autor            Serie       Normalizado
    1 The Name of the W…  Patrick Rothfuss Kingkille…  Sí
```

### Detalle vertical (`--long`)

Un libro por bloque, todos los metadatos:

```
ID:          42
Título:      A Dance with Dragons
Autor:       George R. R. Martin
Serie:       A Song of Ice and Fire
Normalizado: Sí
Fecha:       2024-05-31 14:22
Hash:        a1b2c3...
Ruta:        /home/user/Books/Rubrica/...
```

### Modo avanzado (`--extralong`)

Incluye todo lo anterior más:

```
Tamaño:      2.4 MB

Capítulos:
  1. Prologue
  2. Tyrion I
  3. Daenerys I
  ...
```

---

## Esquema de base de datos

SQLite con tablas principales:

- **`books`**: metadatos, ruta actual, hash SHA-256, flag de normalización
- **`authors`**: nombre del autor
- **`series`**: nombre de la serie
- **`books_fts`**: índice virtual FTS5 (título + autor) para búsqueda full-text
- **`aliases`**: aliases persistentes del CLI/REPL
- **`file_hash`**: hashes de archivos (usado para deduplicación)

Las migraciones se ejecutan automáticamente al inicializar `LibraryDb`.

---

## Dependencias principales

| Crate | Uso |
|---|---|
| `tokio` | Runtime async |
| `sqlx` | Acceso a SQLite con tipado en tiempo de compilación |
| `axum` | Servidor web OPDS |
| `clap` | CLI con subcomandos y flags tipados |
| `rustyline` | REPL interactivo con historial y autocompletado |
| `zip` + `roxmltree` | Parseo de EPUBs (ZIP + OPF/NCX) |
| `sha2` + `hex` | Hash SHA-256 para deduplicación |
| `walkdir` | Exploración recursiva de directorios |
| `gutencore` | Validación estructural de EPUBs (dependencia local) |

---

## Roadmap

- [x] Core backend (DB, importación, búsqueda FTS5)
- [x] CLI con subcomandos y flags
- [x] REPL interactivo con autocompletado
- [x] Sistema de aliases persistentes en DB
- [x] Normalización de archivos en disco
- [x] Servidor OPDS embebido
- [x] Validación de salud editorial
- [x] Filtros avanzados (`--ids`, `--fts` con sintaxis Google-like, `--long`, `--extralong`)
- [ ] Comando `series` (listar series con conteo, similar a `authors`)
- [ ] Extracción mejorada de metadatos (`calibre:series`, `belongs-to-collection`)
- [ ] Sistema de tags y comando `tag`
- [ ] OPDS con navegación jerárquica por autor/serie
- [ ] Colores en output del REPL
- [ ] Shell completion scripts (bash/zsh)

---

## Licencia

MIT o Apache-2.0, a elección del usuario.

---

## Autor

**Pedro Busto** — [@pclbusto](https://github.com/pclbusto)
