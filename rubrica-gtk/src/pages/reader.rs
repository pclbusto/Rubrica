use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use rubrica::library_db::LibraryDb;

pub fn build(db: LibraryDb, book_id: i64, _nav_view: adw::NavigationView) -> adw::NavigationPage {
    let page_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    let header_bar = adw::HeaderBar::new();
    page_box.append(&header_bar);

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .build();

    let status = adw::StatusPage::builder()
        .title("Abrir con…")
        .description("Elegí la aplicación con la que querés leer este libro.")
        .icon_name("document-open-symbolic")
        .vexpand(true)
        .build();

    // Lista de apps disponibles para epub
    let apps_group = adw::PreferencesGroup::new();

    let vbox = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .margin_top(24)
        .margin_bottom(24)
        .build();
    vbox.append(&status);
    vbox.append(&apps_group);

    clamp.set_child(Some(&vbox));
    page_box.append(&clamp);

    let page = adw::NavigationPage::builder()
        .title("Leer")
        .child(&page_box)
        .build();

    // Carga async: título del libro + lista de apps
    {
        let db2 = db.clone();
        let page_weak = page.downgrade();
        let group_weak = apps_group.downgrade();

        glib::MainContext::default().spawn_local(async move {
            let book = match db2.get_book(book_id).await {
                Ok(Some(b)) => b,
                _ => return,
            };

            if let Some(p) = page_weak.upgrade() {
                p.set_title(&book.title);
            }

            // Obtener apps instaladas que soportan epub
            let uri = format!("file://{}", book.current_path);
            let apps = gtk::gio::AppInfo::recommended_for_type("application/epub+zip");

            if let Some(group) = group_weak.upgrade() {
                if apps.is_empty() {
                    // Sin apps registradas — row informativo
                    let row = adw::ActionRow::builder()
                        .title("Sin lectores instalados")
                        .subtitle("Instalá Foliate, Calibre o similar desde GNOME Software.")
                        .build();
                    group.add(&row);
                } else {
                    for app in &apps {
                        let row = build_app_row(app, uri.clone());
                        group.add(&row);
                    }
                }
            }
        });
    }

    page
}

fn build_app_row(app: &gtk::gio::AppInfo, uri: String) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(app.name().as_str())
        .activatable(true)
        .build();

    // Ícono de la app
    if let Some(icon) = app.icon() {
        let img = gtk::Image::builder()
            .gicon(&icon)
            .icon_size(gtk::IconSize::Large)
            .valign(gtk::Align::Center)
            .build();
        row.add_prefix(&img);
    }

    // Flecha →
    let arrow = gtk::Image::builder()
        .icon_name("go-next-symbolic")
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&arrow);

    // Clic → lanzar
    let app = app.clone();
    row.connect_activated(move |_| {
        let _ = app.launch_uris(&[uri.as_str()], None::<&gtk::gio::AppLaunchContext>);
    });

    row
}
