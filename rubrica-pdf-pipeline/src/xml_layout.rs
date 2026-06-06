/// Layout-based PDF structure analysis using pdftohtml XML output.
///
/// Reconstructs chapters from visual/positional properties of text elements:
/// font size ratios, vertical position on page, line fill ratios.
/// Does not rely on text content or language-specific keywords.
use crate::{Chapter, PdfPipelineError};
use regex::Regex;
use roxmltree::Document;
use std::collections::HashMap;
use tracing::info;

// ─── Umbrales ────────────────────────────────────────────────────────────────

/// font_size ≥ body × este ratio → candidato a encabezado o drop cap.
const HEADING_RATIO: f32 = 1.15;

/// Elemento de 1 carácter alfabético a tamaño heading → drop cap decorativo.
/// Se marca como inicio de capítulo pero no se usa como título.
const DROPCAP_ALPHA_MAX_CHARS: usize = 2; // ≤ 2 chars con exactamente 1 alfa → drop cap

/// Zona de encabezado corriente: top < page_h × RUNNING_ZONE (fracción).
const RUNNING_ZONE: f32 = 0.07;

/// Texto que aparece en zona running con gap promedio ≤ este valor (páginas) → elemento corriente.
const RUNNING_GAP_MAX: f32 = 4.0;

/// Mínimo de apariciones para considerar un texto como elemento corriente.
const RUNNING_MIN_OCCURRENCES: usize = 3;

/// Fill ratio de la última línea del cuerpo de una página ≥ este valor → párrafo continúa.
const PARA_CONTINUE_FILL: f32 = 0.78;

/// Gap vertical entre líneas > line_height × este ratio → nuevo párrafo.
const PARA_GAP_RATIO: f32 = 1.55;

/// Tolerancia en px para agrupar elementos de texto en la misma línea lógica.
///
/// Algunos PDFs usan drop caps estilizados donde la capital ("I") está ~6px más arriba
/// que el resto de la palabra ("MPERIAL"), produciendo "IMPERIAL" como dos elementos
/// en Y distintos pero horizontalmente contiguos. Con 10px capturamos estos casos sin
/// fusionar líneas consecutivas del cuerpo (que difieren en ~20-24px de Y).
const LINE_Y_TOLERANCE: f32 = 10.0;

// ─── Tipos internos ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RawElement {
    top: f32,
    left: f32,
    width: f32,
    height: f32,
    font_size: u32,
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
enum LineKind {
    ChapterHeading,
    SceneBreak,
    DropCap,
    Running,
    Body,
}

#[derive(Debug, Clone)]
struct LayoutLine {
    top: f32,
    height: f32,
    text: String,
    font_size: u32,
    /// Ancho total del texto / ancho del área de texto. > 1.0 es posible por redondeo.
    fill_ratio: f32,
    ends_sentence: bool,
    kind: LineKind,
}

#[derive(Debug)]
struct LayoutPage {
    number: u32,
    lines: Vec<LayoutLine>,
}

// ─── Punto de entrada público ─────────────────────────────────────────────────

/// Parsea un XML de `pdftohtml -xml` y devuelve capítulos reconstruidos
/// usando la estructura visual del PDF (tamaño de fuente, posición, fill ratio).
pub fn parse_and_reconstruct(xml: &str) -> Result<Vec<Chapter>, PdfPipelineError> {
    // roxmltree no soporta declaraciones DTD (<!DOCTYPE ...>).
    // pdftohtml siempre las incluye — las eliminamos antes de parsear.
    let xml_clean: std::borrow::Cow<str> = if xml.contains("<!DOCTYPE") {
        let filtered = xml
            .lines()
            .filter(|l| !l.trim_start().starts_with("<!DOCTYPE"))
            .collect::<Vec<_>>()
            .join("\n");
        std::borrow::Cow::Owned(filtered)
    } else {
        std::borrow::Cow::Borrowed(xml)
    };

    let (raw_pages, _) = parse_xml_to_raw(&xml_clean)?;

    if raw_pages.is_empty() {
        return Err(PdfPipelineError::ExtractionFailed(
            "XML sin páginas con texto".into(),
        ));
    }

    // Calcular propiedades globales del documento
    let all_elements: Vec<&RawElement> = raw_pages
        .iter()
        .flat_map(|(_, _, _, els)| els.iter())
        .collect();

    let body_size = body_font_size(&all_elements);
    let (text_left, text_right) = text_area(&all_elements, body_size);
    let text_width = (text_right - text_left).max(1.0);

    info!(
        "body={}pt, text_area=[{:.0}..{:.0}] ({:.0}px)",
        body_size, text_left, text_right, text_width
    );

    let running = detect_running_elements(&raw_pages);
    if !running.is_empty() {
        info!("Elementos corrientes detectados: {:?}", running);
    }

    let scene_re = Regex::new(r"^[\*\-·•–—]{1,3}(\s*[\*\-·•–—]{1,3})+$").unwrap();

    // Construir páginas con líneas clasificadas
    let mut layout_pages: Vec<LayoutPage> = Vec::new();
    for (page_num, page_h, elements) in raw_pages
        .iter()
        .map(|(n, _, h, e)| (*n, *h, e))
    {
        let lines = build_lines(
            elements,
            body_size,
            text_width,
            &running,
            &scene_re,
            page_h,
        );
        layout_pages.push(LayoutPage {
            number: page_num,
            lines,
        });
    }

    let chapters = reconstruct_chapters(&layout_pages, text_width);
    info!("{} capítulos reconstruidos desde XML", chapters.len());
    Ok(chapters)
}

// ─── Parseo XML ───────────────────────────────────────────────────────────────

fn parse_xml_to_raw(
    xml: &str,
) -> Result<
    (
        Vec<(u32, f32, f32, Vec<RawElement>)>, // (page_num, page_w, page_h, elements)
        HashMap<String, u32>,                   // font_id → size
    ),
    PdfPipelineError,
> {
    let doc = Document::parse(xml)
        .map_err(|e| PdfPipelineError::ExtractionFailed(format!("XML: {}", e)))?;

    let mut global_fonts: HashMap<String, u32> = HashMap::new();
    let mut pages: Vec<(u32, f32, f32, Vec<RawElement>)> = Vec::new();

    for node in doc.root_element().children() {
        if node.tag_name().name() != "page" {
            continue;
        }

        let page_num: u32 = node
            .attribute("number")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let page_h: f32 = node
            .attribute("height")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1262.0);
        let page_w: f32 = node
            .attribute("width")
            .and_then(|s| s.parse().ok())
            .unwrap_or(892.0);

        // Acumular fontspecs globalmente (los ids se resan entre páginas)
        for fs in node.children().filter(|n| n.tag_name().name() == "fontspec") {
            let id = fs.attribute("id").unwrap_or("").to_string();
            let size: u32 = fs
                .attribute("size")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            global_fonts.insert(id, size);
        }

        let mut elements: Vec<RawElement> = Vec::new();
        for text_node in node.children().filter(|n| n.tag_name().name() == "text") {
            let top: f32 = text_node
                .attribute("top")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let left: f32 = text_node
                .attribute("left")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let width: f32 = text_node
                .attribute("width")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let height: f32 = text_node
                .attribute("height")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let font_id = text_node.attribute("font").unwrap_or("").to_string();
            let font_size = global_fonts.get(&font_id).copied().unwrap_or(0);
            let text = extract_text(text_node);

            if !text.trim().is_empty() && font_size > 0 {
                elements.push(RawElement {
                    top,
                    left,
                    width,
                    height,
                    font_size,
                    text,
                });
            }
        }

        pages.push((page_num, page_w, page_h, elements));
    }

    Ok((pages, global_fonts))
}

fn extract_text(node: roxmltree::Node) -> String {
    let raw: String = node
        .descendants()
        .filter(|n| n.is_text())
        .map(|n| n.text().unwrap_or(""))
        .collect();
    sanitize_xml(&raw)
}

/// Elimina caracteres inválidos para XML 1.0.
/// XML 1.0 solo acepta: #x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] | [#x10000-#x10FFFF]
/// Los PDFs pueden contener bytes en rangos prohibidos (caracteres de control, surrogates, etc.)
/// que pasan a través de pdftohtml y rompen la renderización en lectores EPUB.
fn sanitize_xml(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            matches!(c, '\t' | '\n' | '\r')
                || ('\u{0020}'..='\u{D7FF}').contains(&c)
                || ('\u{E000}'..='\u{FFFD}').contains(&c)
                || ('\u{10000}'..='\u{10FFFF}').contains(&c)
        })
        .collect()
}

// ─── Propiedades globales del documento ──────────────────────────────────────

/// Tamaño de fuente del cuerpo = el más frecuente ponderado por cantidad de caracteres.
fn body_font_size(elements: &[&RawElement]) -> u32 {
    let mut char_count: HashMap<u32, usize> = HashMap::new();
    for el in elements {
        *char_count.entry(el.font_size).or_default() += el.text.chars().count();
    }
    char_count
        .into_iter()
        .filter(|(size, _)| *size > 0)
        .max_by_key(|(_, count)| *count)
        .map(|(size, _)| size)
        .unwrap_or(12)
}

/// Límites izquierdo y derecho del área de texto, estimados desde las posiciones
/// más frecuentes de los elementos con el font size del cuerpo.
fn text_area(elements: &[&RawElement], body_size: u32) -> (f32, f32) {
    let mode_of = |vals: Vec<i32>| -> f32 {
        if vals.is_empty() {
            return 0.0;
        }
        let mut counts: HashMap<i32, usize> = HashMap::new();
        for v in &vals {
            *counts.entry(*v).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(v, _)| v as f32)
            .unwrap_or(0.0)
    };

    let body_els: Vec<&RawElement> = elements
        .iter()
        .filter(|e| e.font_size == body_size && e.width > 10.0)
        .copied()
        .collect();

    let lefts: Vec<i32> = body_els.iter().map(|e| e.left as i32).collect();
    let rights: Vec<i32> = body_els
        .iter()
        .map(|e| (e.left + e.width) as i32)
        .collect();

    let left = mode_of(lefts);
    let right = mode_of(rights);
    (left, right.max(left + 100.0))
}

// ─── Elementos corrientes (headers/footers) ───────────────────────────────────

/// Detecta textos que son encabezados o pies corrientes.
///
/// Criterio: texto que aparece en la zona superior de la página (top < 7% de page_h)
/// con un gap promedio entre apariciones consecutivas ≤ RUNNING_GAP_MAX páginas.
/// Esto distingue headers que aparecen en cada página de encabezados de capítulo
/// que se repiten con un gap mucho mayor (≈ páginas por capítulo).
fn detect_running_elements(pages: &[(u32, f32, f32, Vec<RawElement>)]) -> std::collections::HashSet<String> {
    // Acumular listas de páginas donde aparece cada texto en zona running
    let mut text_pages: HashMap<String, Vec<u32>> = HashMap::new();

    for (page_num, _, page_h, elements) in pages {
        let zone_limit = page_h * RUNNING_ZONE;
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for el in elements {
            if el.top >= zone_limit {
                continue;
            }
            let t = el.text.trim().to_string();
            // Excluir números puros (números de página) — se manejan por separado
            if t.is_empty() || t.chars().all(|c| c.is_ascii_digit() || c.is_whitespace()) {
                continue;
            }
            seen.insert(t);
        }

        for t in seen {
            text_pages.entry(t).or_default().push(*page_num);
        }
    }

    let mut running = std::collections::HashSet::new();
    for (text, page_nums) in &text_pages {
        if page_nums.len() < RUNNING_MIN_OCCURRENCES {
            continue;
        }
        let mut sorted = page_nums.clone();
        sorted.sort_unstable();

        let gaps: Vec<f32> = sorted
            .windows(2)
            .map(|w| (w[1] - w[0]) as f32)
            .collect();

        let avg_gap = gaps.iter().sum::<f32>() / gaps.len() as f32;

        if avg_gap <= RUNNING_GAP_MAX {
            running.insert(text.clone());
        }
    }

    running
}

// ─── Construcción de líneas lógicas ──────────────────────────────────────────

/// Agrupa los elementos de texto de una página en líneas lógicas (mismo top ± tolerancia),
/// calcula el fill ratio de cada línea y la clasifica.
fn build_lines(
    elements: &[RawElement],
    body_size: u32,
    text_width: f32,
    running: &std::collections::HashSet<String>,
    scene_re: &Regex,
    page_h: f32,
) -> Vec<LayoutLine> {
    if elements.is_empty() {
        return vec![];
    }

    // Agrupar por top (tolerancia LINE_Y_TOLERANCE px)
    let mut groups: Vec<(f32, Vec<&RawElement>)> = Vec::new();
    'outer: for el in elements {
        for (group_top, group) in &mut groups {
            if (el.top - *group_top).abs() <= LINE_Y_TOLERANCE {
                group.push(el);
                continue 'outer;
            }
        }
        groups.push((el.top, vec![el]));
    }
    groups.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let running_zone = page_h * RUNNING_ZONE;

    groups
        .into_iter()
        .filter_map(|(top, group)| {
            // Ordenar elementos de izquierda a derecha dentro de la línea
            let mut sorted_group = group;
            sorted_group.sort_by(|a, b| a.left.partial_cmp(&b.left).unwrap());

            // Texto completo de la línea (concatenar con espacio si hay gap horizontal real).
            // Umbral de 4px: gaps menores son artefactos del renderizado (ej. drop caps
            // donde la capital y el resto de la palabra difieren en pocos píxeles de left).
            let mut text = String::new();
            let mut prev_right = f32::NEG_INFINITY;
            for el in &sorted_group {
                if !text.is_empty() && el.left > prev_right + 4.0 {
                    text.push(' ');
                }
                text.push_str(&el.text);
                prev_right = el.left + el.width;
            }
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }

            // Font size dominante: el más frecuente (por chars) en el grupo
            let font_size = {
                let mut fc: HashMap<u32, usize> = HashMap::new();
                for el in &sorted_group {
                    *fc.entry(el.font_size).or_default() += el.text.chars().count();
                }
                fc.into_iter().max_by_key(|(_, c)| *c).map(|(s, _)| s).unwrap_or(body_size)
            };

            // Altura representativa (máxima del grupo)
            let height = sorted_group.iter().map(|e| e.height).fold(0.0f32, f32::max);

            // Fill ratio: suma de anchos de elementos / ancho del área de texto
            let total_width: f32 = sorted_group.iter().map(|e| e.width).sum();
            let fill_ratio = total_width / text_width;

            // ¿Termina en puntuación de fin de oración?
            let ends_sentence = text
                .trim_end_matches(|c| c == '"' || c == '\'' || c == '»' || c == '"')
                .ends_with(|c| matches!(c, '.' | '?' | '!' | ':' | ';' | '…'));

            // Clasificar la línea
            let kind = classify_line(
                &text,
                font_size,
                body_size,
                top,
                running_zone,
                running,
                scene_re,
            );

            Some(LayoutLine {
                top,
                height,
                text,
                font_size,
                fill_ratio,
                ends_sentence,
                kind,
            })
        })
        .collect()
}

fn classify_line(
    text: &str,
    font_size: u32,
    body_size: u32,
    top: f32,
    running_zone: f32,
    running: &std::collections::HashSet<String>,
    scene_re: &Regex,
) -> LineKind {
    let t = text.trim();

    // 1. Elemento corriente (header/footer repetido)
    if top < running_zone && running.contains(t) {
        return LineKind::Running;
    }

    // 2. Números de página: número puro en zona running
    if top < running_zone && t.chars().all(|c| c.is_ascii_digit()) {
        return LineKind::Running;
    }

    // 3. Tamaño body o menor → texto del cuerpo
    let heading_threshold = (body_size as f32 * HEADING_RATIO) as u32;
    if font_size < heading_threshold {
        return LineKind::Body;
    }

    // A partir de aquí: font_size ≥ heading_threshold

    // 4. Scene break: patrón de separador de escena (puede venir en tamaño heading en algunos PDFs)
    if scene_re.is_match(t) {
        return LineKind::SceneBreak;
    }

    // 5. Drop cap: ≤ DROPCAP_ALPHA_MAX_CHARS chars con exactamente 1 carácter alfabético
    let alpha_count = t.chars().filter(|c| c.is_alphabetic()).count();
    let char_count = t.chars().count();
    if char_count <= DROPCAP_ALPHA_MAX_CHARS && alpha_count == 1 {
        return LineKind::DropCap;
    }

    // 6. Encabezado de capítulo / sección
    LineKind::ChapterHeading
}

// ─── Reconstrucción de capítulos ──────────────────────────────────────────────

fn reconstruct_chapters(pages: &[LayoutPage], text_width: f32) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut current_title = String::new();
    let mut current_number: Option<u32> = None;
    let mut current_body = String::new();
    let mut auto_chapter_num = 0u32;

    // Estado de continuación de párrafo entre páginas
    let mut prev_continues = false;
    // Dentro de una página: top de la línea anterior y su height (para detectar gaps de párrafo)
    let mut prev_line_top: Option<f32> = None;
    let mut prev_line_height: f32 = 0.0;

    for page in pages.iter() {
        // Determinar si la última línea de esta página continúa a la siguiente
        let this_page_continues = page_last_body_continues(&page.lines, text_width);

        // Primera línea de esta página: ¿continúa párrafo anterior?
        let mut is_first_body = true;

        for line in &page.lines {
            match line.kind {
                LineKind::Running => {
                    // Ignorar: header/footer corriente
                }
                LineKind::DropCap => {
                    // Marca inicio de capítulo pero no tiene título propio
                    if !current_body.trim().is_empty() || !current_title.is_empty() {
                        push_chapter(
                            &mut chapters,
                            current_number,
                            std::mem::take(&mut current_title),
                            std::mem::take(&mut current_body),
                        );
                    }
                    // El drop cap podría ser la primera letra de la primera palabra.
                    // Lo incluimos como primer carácter del cuerpo del nuevo capítulo.
                    auto_chapter_num += 1;
                    current_number = Some(auto_chapter_num);
                    current_title = String::new(); // sin título
                    // Prepend el carácter decorativo al cuerpo
                    current_body = line.text.trim().to_string();
                    prev_continues = false;
                    prev_line_top = None;
                    is_first_body = false;
                }
                LineKind::ChapterHeading => {
                    if !current_title.is_empty() && current_body.trim().is_empty() {
                        // Título multi-línea: heading consecutivo sin cuerpo entre ellos.
                        // Fusionar en lugar de crear un capítulo nuevo.
                        current_title.push('\n');
                        current_title.push_str(line.text.trim());
                        if current_number.is_none() {
                            current_number = extract_number(line.text.trim());
                        }
                    } else {
                        // Guardar capítulo anterior y empezar uno nuevo
                        if !current_body.trim().is_empty() || !current_title.is_empty() {
                            push_chapter(
                                &mut chapters,
                                current_number,
                                std::mem::take(&mut current_title),
                                std::mem::take(&mut current_body),
                            );
                        }
                        current_title = line.text.trim().to_string();
                        current_number = extract_number(&current_title);
                        current_body = String::new();
                        prev_continues = false;
                        prev_line_top = None;
                        is_first_body = true;
                    }
                }
                LineKind::SceneBreak => {
                    // Añadir separador de escena al cuerpo actual
                    if !current_body.is_empty() {
                        current_body.push_str("\n\n* * *\n\n");
                    }
                    prev_line_top = None;
                    prev_continues = false;
                }
                LineKind::Body => {
                    let text = line.text.trim();

                    // Determinar si esta línea continúa directamente la anterior
                    // (sin salto de párrafo)
                    let is_drop_cap_body = is_drop_cap_remainder(&current_body, text);

                    let continuation = is_drop_cap_body
                        || if is_first_body && prev_continues {
                            true
                        } else if let Some(pt) = prev_line_top {
                            let gap = line.top - pt;
                            gap <= prev_line_height * PARA_GAP_RATIO
                        } else {
                            false
                        };

                    if current_body.is_empty() {
                        current_body.push_str(text);
                    } else if continuation {
                        join_to_paragraph(&mut current_body, text);
                    } else {
                        current_body.push_str("\n\n");
                        current_body.push_str(text);
                    }

                    prev_line_top = Some(line.top);
                    prev_line_height = line.height.max(1.0);
                    is_first_body = false;
                }
            }
        }

        // Al final de la página, resetear el contexto de línea para la siguiente
        prev_continues = this_page_continues;
        // El top previo no se lleva entre páginas (el gap cross-page lo gestiona prev_continues)
        prev_line_top = if prev_continues { None } else { None };
    }

    // Capítulo final
    if !current_body.trim().is_empty() || !current_title.is_empty() {
        push_chapter(
            &mut chapters,
            current_number,
            current_title,
            current_body,
        );
    }

    // Si no se detectó ningún encabezado de capítulo, todo el texto va a un solo capítulo
    if chapters.is_empty() {
        // No se detectó estructura → devolver vacío (el caller puede aplicar fallback)
        return vec![];
    }

    chapters
}

/// Determina si la última línea de cuerpo de la página indica que el párrafo continúa.
fn page_last_body_continues(lines: &[LayoutLine], _text_width: f32) -> bool {
    let last_body = lines
        .iter()
        .rev()
        .find(|l| l.kind == LineKind::Body);

    match last_body {
        Some(line) => line.fill_ratio >= PARA_CONTINUE_FILL && !line.ends_sentence,
        None => false,
    }
}

/// Detecta si `body` es un drop cap suelto (1 letra mayúscula) y `text` es su continuación.
/// Ej: body="T", text="he quick brown fox" → true → deben unirse sin espacio como "The quick..."
fn is_drop_cap_remainder(body: &str, text: &str) -> bool {
    let mut chars = body.chars();
    let Some(first) = chars.next() else { return false };
    if chars.next().is_some() { return false } // más de 1 carácter → no es drop cap
    first.is_uppercase()
        && text.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
}

/// Une una línea al párrafo en curso, manejando guiones de corte y drop caps.
fn join_to_paragraph(body: &mut String, text: &str) {
    if body.ends_with('-')
        && !body.ends_with("—")
        && !body.ends_with("–")
        && text.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
    {
        // Guion de corte de palabra: eliminar guion y unir directamente
        body.pop();
        body.push_str(text);
    } else if is_drop_cap_remainder(body, text) {
        // Drop cap suelto + continuación en minúscula: "T" + "he quick" → "The quick"
        body.push_str(text);
    } else {
        body.push(' ');
        body.push_str(text);
    }
}

fn push_chapter(
    chapters: &mut Vec<Chapter>,
    number: Option<u32>,
    title: String,
    body: String,
) {
    let body = body.trim().to_string();
    if body.is_empty() && title.is_empty() {
        return;
    }
    chapters.push(Chapter {
        number,
        title,
        body,
    });
}

fn extract_number(text: &str) -> Option<u32> {
    let re = Regex::new(r"(\d+)").unwrap();
    re.captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}
