use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use rubrica::library_db::LibraryDb;
use rubrica::analytics::Analytics;

/// Construye un AdwNavigationPage con la ficha completa del libro.
pub fn build(db: LibraryDb, book_id: i64, nav_view: adw::NavigationView) -> adw::NavigationPage {
    let page_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    let header_bar = adw::HeaderBar::new();

    // Botón "Leer" en el header de detalle
    let btn_read = gtk::Button::builder()
        .label("Leer")
        .css_classes(vec!["suggested-action"])
        .build();
    {
        let db2 = db.clone();
        let nav2 = nav_view.clone();
        btn_read.connect_clicked(move |_| {
            let page = crate::pages::reader::build(db2.clone(), book_id, nav2.clone());
            nav2.push(&page);
        });
    }
    header_bar.pack_end(&btn_read);

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(720)
        .margin_top(24)
        .margin_bottom(32)
        .margin_start(16)
        .margin_end(16)
        .build();

    let content = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(24)
        .build();

    // ── Sección superior: portada + metadatos ──────────────────────────
    let top_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(24)
        .build();

    // Portada grande
    let picture = gtk::Picture::builder()
        .width_request(160)
        .height_request(240)
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .css_classes(vec!["card"])
        .build();

    // Metadatos a la derecha de la portada
    let meta_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .vexpand(true)
        .valign(gtk::Align::Start)
        .build();

    let title_lbl = gtk::Label::builder()
        .css_classes(vec!["title-1"])
        .xalign(0.0)
        .wrap(true)
        .build();
    let author_lbl = gtk::Label::builder()
        .css_classes(vec!["title-3"])
        .xalign(0.0)
        .build();

    // Tags de idioma y saga (placeholders, se llenarán async)
    let tags_box = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let series_tag = gtk::Label::builder()
        .css_classes(vec!["tag", "accent"])
        .visible(false)
        .build();
    tags_box.append(&series_tag);

    // UUID / hash
    let hash_lbl = gtk::Label::builder()
        .css_classes(vec!["dim-label", "caption"])
        .xalign(0.0)
        .selectable(true)
        .build();
    let path_lbl = gtk::Label::builder()
        .css_classes(vec!["dim-label", "caption"])
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();

    meta_box.append(&title_lbl);
    meta_box.append(&author_lbl);
    meta_box.append(&tags_box);
    meta_box.append(&hash_lbl);
    meta_box.append(&path_lbl);

    top_row.append(&picture);
    top_row.append(&meta_box);
    content.append(&top_row);

    // ── Grupo: edición inline de metadatos ─────────────────────────────
    let edit_group = adw::PreferencesGroup::builder()
        .title("Metadatos")
        .build();

    let row_title = adw::EntryRow::builder()
        .title("Título")
        .build();
    let row_author = adw::EntryRow::builder()
        .title("Autor")
        .build();
    let row_series = adw::EntryRow::builder()
        .title("Saga")
        .build();

    edit_group.add(&row_title);
    edit_group.add(&row_author);
    edit_group.add(&row_series);

    // Botón guardar metadatos
    let btn_save = gtk::Button::builder()
        .label("Guardar cambios")
        .css_classes(vec!["suggested-action"])
        .halign(gtk::Align::End)
        .build();
    {
        let db2 = db.clone();
        let rt = row_title.clone();
        let ra = row_author.clone();
        let rs = row_series.clone();
        btn_save.connect_clicked(move |_| {
            let new_title = rt.text().to_string();
            let new_author = ra.text().to_string();
            let _new_series = rs.text().to_string();
            let db3 = db2.clone();
            glib::MainContext::default().spawn_local(async move {
                let t = if new_title.is_empty() { None } else { Some(new_title.as_str()) };
                let a = if new_author.is_empty() { None } else { Some(new_author.as_str()) };
                if let Err(e) = db3.update_book(book_id, t, None, a, None).await {
                    eprintln!("Error guardando metadatos: {e}");
                }
            });
        });
    }

    content.append(&edit_group);
    content.append(&btn_save);

    // ── Grupo: Auditoría editorial ─────────────────────────────────────
    let health_group = adw::PreferencesGroup::builder()
        .title("Auditoría Editorial")
        .build();

    let row_health = adw::ActionRow::builder()
        .title("Salud del archivo")
        .subtitle("Analizando…")
        .build();

    health_group.add(&row_health);
    content.append(&health_group);

    clamp.set_child(Some(&content));
    scroll.set_child(Some(&clamp));
    page_box.append(&header_bar);
    page_box.append(&scroll);

    let page = adw::NavigationPage::builder()
        .title("Ficha del libro")
        .child(&page_box)
        .build();

    // ── Carga async del libro ──────────────────────────────────────────
    {
        let db2 = db.clone();
        let pic_weak = picture.downgrade();
        let title_weak = title_lbl.downgrade();
        let author_weak = author_lbl.downgrade();
        let hash_weak = hash_lbl.downgrade();
        let path_weak = path_lbl.downgrade();
        let series_weak = series_tag.downgrade();
        let page_weak = page.downgrade();
        let rt_weak = row_title.downgrade();
        let ra_weak = row_author.downgrade();
        let rs_weak = row_series.downgrade();
        let rh_weak = row_health.downgrade();

        glib::MainContext::default().spawn_local(async move {
            let book = match db2.get_book(book_id).await {
                Ok(Some(b)) => b,
                _ => return,
            };

            if let Some(p) = page_weak.upgrade() {
                p.set_title(&book.title);
            }
            if let Some(l) = title_weak.upgrade() { l.set_text(&book.title); }
            if let Some(l) = author_weak.upgrade() { l.set_text(&book.author_name); }
            if let Some(l) = hash_weak.upgrade() {
                l.set_text(&format!("Ruta: {}", book.current_path));
            }
            if let Some(l) = path_weak.upgrade() {
                let date = book.date_added.format("%d/%m/%Y").to_string();
                l.set_text(&format!("Añadido: {}", date));
            }
            if let Some(t) = series_weak.upgrade() {
                if let Some(s) = &book.series_name {
                    t.set_text(s);
                    t.set_visible(true);
                }
            }

            // Pre-rellenar campos de edición
            if let Some(r) = rt_weak.upgrade() { r.set_text(&book.title); }
            if let Some(r) = ra_weak.upgrade() { r.set_text(&book.author_name); }
            if let Some(r) = rs_weak.upgrade() {
                r.set_text(book.series_name.as_deref().unwrap_or(""));
            }

            // Cargar portada (cover_href puede ser None; cover::load lo detecta en el EPUB)
            if let Some(pic) = pic_weak.upgrade() {
                if let Some(px) = crate::cover::load(book.current_path.clone(), book.cover_href.clone(), 200, 300).await {
                    let tex = gtk::gdk::Texture::for_pixbuf(&px);
                    pic.set_paintable(Some(tex.upcast_ref::<gtk::gdk::Paintable>()));
                }
            }

            // Validación de links
            if let Some(rh) = rh_weak.upgrade() {
                match Analytics::validate_links(book_id, &book.current_path).await {
                    Ok(report) => {
                        if report.broken_links == 0 {
                            rh.set_subtitle("✓ Todos los enlaces son válidos");
                        } else {
                            rh.set_subtitle(&format!(
                                "⚠ {} enlace(s) roto(s) detectado(s)",
                                report.broken_links
                            ));
                        }
                    }
                    Err(_) => {
                        rh.set_subtitle("No se pudo analizar el archivo");
                    }
                }
            }
        });
    }

    page
}
