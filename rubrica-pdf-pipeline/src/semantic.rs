/// Convierte capítulos estructurados a fragmentos XHTML para el ensamblador EPUB.
use crate::Chapter;
use regex::Regex;

const COMMON_WORDS: &[&str] = &[
    "el", "la", "los", "las", "un", "una", "unos", "unas", "y", "e", "o", "u", "de", "del",
    "al", "en", "con", "por", "para", "que", "se", "su", "sus", "es", "son", "fue", "fueron",
    "ha", "han", "no", "si", "sí", "pero", "como", "más", "muy", "todo", "todos", "the", "and",
    "of", "to", "a", "in", "for", "is", "on", "that",
];

pub struct SemanticStructurer;

impl SemanticStructurer {
    pub fn structure_chapter(chapter: &Chapter) -> String {
        let mut xhtml = String::new();
        if !chapter.title.is_empty() {
            // Títulos multi-línea (separados por \n) → cada línea escapeada + <br/>
            let title_html = chapter
                .title
                .lines()
                .map(|l| xml_escape(l.trim()))
                .collect::<Vec<_>>()
                .join("<br/>");
            xhtml.push_str(&format!("<h1>{}</h1>\n", title_html));
        }
        xhtml.push_str(&Self::structure(&chapter.body));
        xhtml
    }

    pub fn structure(text: &str) -> String {
        let blocks: Vec<&str> = text
            .split("\n\n")
            .map(|b| b.trim())
            .filter(|b| !b.is_empty())
            .collect();
        Self::blocks_to_xhtml(&blocks)
    }

    fn blocks_to_xhtml(blocks: &[&str]) -> String {
        let mut xhtml = String::new();

        for block in blocks {
            let first_char = block.chars().next().unwrap_or(' ');

            if block.trim() == "* * *" || block.trim() == "***" {
                xhtml.push_str("<hr/>\n");
                continue;
            }

            if first_char == '—' || first_char == '–' {
                let norm = normalize_spaces(block);
                xhtml.push_str(&format!(
                    "<p class=\"dialogue\">{}</p>\n",
                    xml_escape(&norm)
                ));
                continue;
            }

            if first_char == '"' || first_char == '«' {
                let norm = normalize_spaces(block);
                xhtml.push_str(&format!(
                    "<blockquote><p>{}</p></blockquote>\n",
                    xml_escape(&norm)
                ));
                continue;
            }

            let normalized = normalize_spaces(
                &block.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" "),
            );
            xhtml.push_str(&format!("<p>{}</p>\n", xml_escape(&normalized)));
        }

        xhtml
    }

    #[allow(dead_code)]
    fn is_common_word(w: &str) -> bool {
        COMMON_WORDS.contains(&w.to_lowercase().as_str())
    }
}

fn normalize_spaces(s: &str) -> String {
    let re = Regex::new(r"[ \t]{2,}").unwrap();
    re.replace_all(s.trim(), " ").into_owned()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
