use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use rubrica::library_db::LibraryDb;
use rubrica::pipeline::{Pipeline, ImportStatus};
use crate::pages;

pub fn build_ui(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Rúbrica")
        .default_width(1200)
        .default_height(800)
        .build();

    let nav_view = adw::NavigationView::new();
    window.set_content(Some(&nav_view));

    // Ruta canónica compartida con el CLI
    let db_path = rubrica::default_db_url();

    let nav_clone = nav_view.clone();
    glib::MainContext::default().spawn_local(async move {
        match LibraryDb::new(&db_path).await {
            Ok(db) => build_main_page(&nav_clone, db).await,
            Err(e) => {
                let status = adw::StatusPage::builder()
                    .title("Error de base de datos")
                    .description(&e.to_string())
                    .icon_name("dialog-error-symbolic")
                    .build();
                let page = adw::NavigationPage::builder()
                    .title("Error")
                    .child(&status)
                    .build();
                nav_clone.push(&page);
            }
        }
    });

    window.present();
}

async fn build_main_page(nav_view: &adw::NavigationView, db: LibraryDb) {
    // ── Layout principal ──────────────────────────────────────────────
    let main_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    // ── ViewStack (tres pestañas) ──────────────────────────────────────
    let stack = adw::ViewStack::new();
    let view_switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    // ── SearchBar + SearchEntry ────────────────────────────────────────
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Buscar en la biblioteca…")
        .hexpand(true)
        .build();
    let search_clamp = adw::Clamp::builder()
        .maximum_size(600)
        .child(&search_entry)
        .build();
    let search_bar = gtk::SearchBar::builder()
        .show_close_button(true)
        .child(&search_clamp)
        .build();
    // Captura keystrokes desde la ventana directamente
    // (se conectará al window después de presentarlo)

    // ── Páginas del stack ──────────────────────────────────────────────
    let (grid_widget, reload_grid) = pages::grid::build(db.clone(), nav_view.clone(), search_entry.clone());
    let (list_widget, reload_list) = pages::list::build(db.clone(), nav_view.clone());
    let stats_widget = pages::stats::build(db.clone());

    let p_grid = stack.add_titled(&grid_widget, Some("grid"), "Biblioteca");
    p_grid.set_icon_name(Some("view-grid-symbolic"));

    let p_list = stack.add_titled(&list_widget, Some("list"), "Lista");
    p_list.set_icon_name(Some("view-list-symbolic"));

    let p_stats = stack.add_titled(&stats_widget, Some("stats"), "Estadísticas");
    p_stats.set_icon_name(Some("utilities-system-monitor-symbolic"));

    // ── HeaderBar ──────────────────────────────────────────────────────
    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&view_switcher));

    // Botón "+"
    let btn_add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Importar ePub")
        .build();
    header_bar.pack_start(&btn_add);

    // Botón búsqueda
    let btn_search = gtk::Button::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Buscar")
        .build();
    {
        let sb = search_bar.clone();
        btn_search.connect_clicked(move |_| {
            let active = !sb.is_search_mode();
            sb.set_search_mode(active);
            if active { /* el search_entry recibe foco via key-capture */ }
        });
    }
    header_bar.pack_end(&btn_search);

    // Botón menú / configuración
    let btn_menu = gtk::Button::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Configuración")
        .build();
    {
        let db2 = db.clone();
        btn_menu.connect_clicked(move |btn| {
            let parent = btn.root().and_downcast::<gtk::Window>();
            let win = pages::settings::build_window(db2.clone());
            win.set_transient_for(parent.as_ref());
            win.present();
        });
    }
    header_bar.pack_end(&btn_menu);

    // ── Importación de ePubs ───────────────────────────────────────────
    {
        let db2 = db.clone();
        let reload_grid = reload_grid.clone();
        let reload_list = reload_list.clone();
        btn_add.connect_clicked(move |btn| {
            let parent = btn.root().and_downcast::<gtk::Window>();
            let db3 = db2.clone();
            let reload_grid = reload_grid.clone();
            let reload_list = reload_list.clone();
            let reload = move || { reload_grid(); reload_list(); };

            let filters = gtk::FileFilter::new();
            filters.set_name(Some("ePub"));
            filters.add_mime_type("application/epub+zip");
            filters.add_pattern("*.epub");

            let filter_list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filter_list.append(&filters);

            let dialog = gtk::FileDialog::builder()
                .title("Importar ePub")
                .filters(&filter_list)
                .build();

            dialog.open_multiple(
                parent.as_ref(),
                None::<&gtk::gio::Cancellable>,
                move |result| {
                    if let Ok(files) = result {
                        let n = files.n_items();
                        let db4 = db3.clone();
                        let reload = reload.clone();
                        glib::MainContext::default().spawn_local(async move {
                            let mut imported = 0u32;
                            for i in 0..n {
                                if let Some(file) = files.item(i).and_downcast::<gtk::gio::File>() {
                                    if let Some(path) = file.path() {
                                        let path_str = path.to_string_lossy().to_string();
                                        match Pipeline::import_file(&db4, path_str.clone()).await {
                                            Ok(ImportStatus::Imported) => imported += 1,
                                            Ok(ImportStatus::Duplicate { existing_id }) => {
                                                eprintln!("Duplicado (ID {existing_id}): {path_str}");
                                            }
                                            Err(e) => eprintln!("Error: {e}"),
                                        }
                                    }
                                }
                            }
                            if imported > 0 {
                                reload();
                            }
                        });
                    }
                },
            );
        });
    }

    // ── Ensamblado final ───────────────────────────────────────────────
    main_box.append(&header_bar);
    main_box.append(&search_bar);
    main_box.append(&stack);

    let main_page = adw::NavigationPage::builder()
        .title("Rúbrica")
        .tag("main")
        .child(&main_box)
        .build();

    nav_view.push(&main_page);

    // Conectar key capture al window después de que esté montado
    let sb = search_bar.clone();
    glib::idle_add_local_once(move || {
        if let Some(root) = sb.root() {
            sb.set_key_capture_widget(Some(&root));
        }
    });
}
