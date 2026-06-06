use gtk::prelude::*;
use gtk::glib;

/// Genera un widget DrawingArea que dibuja un código QR con Cairo.
pub fn make_widget(data: &str) -> gtk::DrawingArea {
    use qrcode::{QrCode, Color};

    let modules: Vec<Vec<bool>> = match QrCode::new(data.as_bytes()) {
        Ok(code) => {
            let w = code.width();
            (0..w)
                .map(|y| (0..w).map(|x| code[(y, x)] == Color::Dark).collect())
                .collect()
        }
        Err(_) => vec![],
    };

    let size = modules.len();
    let scale = 8i32;
    let border = 1; // módulos de borde
    let dim = ((size + border * 2) as i32) * scale;

    let area = gtk::DrawingArea::builder()
        .content_width(dim)
        .content_height(dim)
        .build();

    area.set_draw_func(move |_, cr, _w, _h| {
        let s = scale as f64;
        let b = (border as f64) * s;

        // Fondo blanco
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().ok();

        // Módulos oscuros
        cr.set_source_rgb(0.0, 0.0, 0.0);
        for (y, row) in modules.iter().enumerate() {
            for (x, &dark) in row.iter().enumerate() {
                if dark {
                    cr.rectangle(b + x as f64 * s, b + y as f64 * s, s, s);
                    cr.fill().ok();
                }
            }
        }
    });

    area
}

/// Obtiene la dirección IP local (primera interfaz no-loopback).
pub fn local_ip() -> String {
    // Abrimos un socket UDP sin enviarlo; el SO completa la dirección local.
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}
