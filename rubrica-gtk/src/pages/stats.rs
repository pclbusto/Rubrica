use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use rubrica::library_db::LibraryDb;
use rubrica::analytics::Analytics;

pub fn build(db: LibraryDb) -> gtk::Widget {
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(700)
        .margin_top(32)
        .margin_bottom(32)
        .margin_start(16)
        .margin_end(16)
        .build();

    let vbox = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(24)
        .build();

    // Título de sección
    let title = gtk::Label::builder()
        .label("Estadísticas de la Biblioteca")
        .css_classes(vec!["title-2"])
        .xalign(0.0)
        .build();
    vbox.append(&title);

    // Grupo: métricas globales
    let group = adw::PreferencesGroup::builder()
        .title("Resumen")
        .build();

    let row_books = adw::ActionRow::builder()
        .title("Total de libros")
        .subtitle("Cargando…")
        .build();
    let row_time = adw::ActionRow::builder()
        .title("Tiempo total de lectura")
        .subtitle("Cargando…")
        .build();

    group.add(&row_books);
    group.add(&row_time);
    vbox.append(&group);

    // Grupo: autores y series
    let group2 = adw::PreferencesGroup::builder()
        .title("Colecciones")
        .build();

    let row_authors = adw::ActionRow::builder()
        .title("Autores")
        .subtitle("Cargando…")
        .build();
    let row_series = adw::ActionRow::builder()
        .title("Sagas / Series")
        .subtitle("Cargando…")
        .build();
    let row_tags = adw::ActionRow::builder()
        .title("Etiquetas")
        .subtitle("Cargando…")
        .build();

    group2.add(&row_authors);
    group2.add(&row_series);
    group2.add(&row_tags);
    vbox.append(&group2);

    clamp.set_child(Some(&vbox));
    scroll.set_child(Some(&clamp));

    // Carga async de métricas
    {
        let db2 = db.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(metrics) = Analytics::get_global_metrics(&db2).await {
                row_books.set_subtitle(&metrics.total_books.to_string());
                let mins = metrics.total_reading_time_secs / 60;
                let hours = mins / 60;
                let label = if hours > 0 {
                    format!("{h}h {m}m", h = hours, m = mins % 60)
                } else {
                    format!("{m} minutos", m = mins)
                };
                row_time.set_subtitle(&label);
            }

            if let Ok(authors) = db2.list_authors().await {
                row_authors.set_subtitle(&authors.len().to_string());
            }
            if let Ok(series) = db2.list_series().await {
                row_series.set_subtitle(&series.len().to_string());
            }
            if let Ok(tags) = db2.list_tags().await {
                row_tags.set_subtitle(&tags.len().to_string());
            }
        });
    }

    scroll.upcast()
}
