use gdk_pixbuf::{Pixbuf, PixbufLoader, InterpType};
use gdk_pixbuf::prelude::*;

/// Carga y escala la portada de un EPUB para mostrarse en un área `max_w × max_h`.
/// Mantiene el aspect ratio; el resultado nunca supera esas dimensiones.
/// Devuelve None si el EPUB no tiene portada o si falla la decodificación.
pub async fn load(
    epub_path: String,
    cover_href: Option<String>,
    max_w: i32,
    max_h: i32,
) -> Option<Pixbuf> {
    let bytes = tokio::task::spawn_blocking(move || extract_bytes(&epub_path, cover_href))
        .await
        .ok()??;

    let px = decode_pixbuf(&bytes)?;
    scale_to_fit(&px, max_w, max_h)
}

fn extract_bytes(epub_path: &str, cover_href: Option<String>) -> Option<Vec<u8>> {
    let core = gutencore::GutenCore::open_epub(epub_path).ok()?;

    let href = match cover_href {
        Some(h) => h,
        None => core.get_cover_image()?.href.clone(),
    };

    let opf_dir = core.opf_dir.as_ref()?;
    let abs = opf_dir.join(&href);
    let bytes = std::fs::read(&abs).ok();
    drop(core);
    bytes
}

fn decode_pixbuf(bytes: &[u8]) -> Option<Pixbuf> {
    let loader = PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

fn scale_to_fit(px: &Pixbuf, max_w: i32, max_h: i32) -> Option<Pixbuf> {
    let ow = px.width() as f64;
    let oh = px.height() as f64;
    let scale = f64::min(max_w as f64 / ow, max_h as f64 / oh);
    let sw = (ow * scale).round() as i32;
    let sh = (oh * scale).round() as i32;
    px.scale_simple(sw.max(1), sh.max(1), InterpType::Bilinear)
}
