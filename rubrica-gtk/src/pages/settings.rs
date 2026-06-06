use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use rubrica::library_db::LibraryDb;
use rubrica::sync::SyncSubsystem;

pub fn build_window(db: LibraryDb) -> adw::PreferencesWindow {
    let win = adw::PreferencesWindow::builder()
        .title("Configuración")
        .default_width(640)
        .default_height(500)
        .build();

    win.add(&page_library());
    win.add(&page_sync(db));

    win
}

fn page_library() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Biblioteca")
        .icon_name("folder-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Directorio Raíz")
        .description("Los ePubs normalizados se almacenan aquí.")
        .build();

    // Mostrar ruta XDG actual
    let data_dir = glib::user_data_dir();
    let lib_path = data_dir.join("rubrica");
    let row_dir = adw::ActionRow::builder()
        .title("Ruta actual")
        .subtitle(lib_path.to_string_lossy().as_ref())
        .build();
    let btn_open_dir = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Abrir en gestor de archivos")
        .valign(gtk::Align::Center)
        .css_classes(vec!["flat"])
        .build();
    {
        let path = lib_path.clone();
        btn_open_dir.connect_clicked(move |_| {
            let uri = format!("file://{}", path.display());
            gtk::gio::AppInfo::launch_default_for_uri_async(
                &uri,
                None::<&gtk::gio::AppLaunchContext>,
                None::<&gtk::gio::Cancellable>,
                |_| {},
            );
        });
    }
    row_dir.add_suffix(&btn_open_dir);
    group.add(&row_dir);

    // Interruptores de automatización
    let group2 = adw::PreferencesGroup::builder()
        .title("Automatización")
        .build();

    let sw_auto_org = adw::SwitchRow::builder()
        .title("Organizar al importar")
        .subtitle("Mover el ePub a la ruta normalizada automáticamente")
        .active(false)
        .build();
    let sw_bg_scan = adw::SwitchRow::builder()
        .title("Escaneo en segundo plano")
        .subtitle("Reindexar metadatos al iniciar la app")
        .active(true)
        .build();

    group2.add(&sw_auto_org);
    group2.add(&sw_bg_scan);

    page.add(&group);
    page.add(&group2);
    page
}

fn page_sync(db: LibraryDb) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Dispositivos")
        .icon_name("phone-symbolic")
        .build();

    // ── MTP (USB) ──────────────────────────────────────────────────────
    let group_mtp = adw::PreferencesGroup::builder()
        .title("USB / MTP (Onyx Boox)")
        .description("Conecta tu tableta por USB para sincronizar.")
        .build();

    let row_mtp = adw::ActionRow::builder()
        .title("Estado del dispositivo")
        .subtitle("Sin dispositivo detectado")
        .build();
    let btn_sync = gtk::Button::builder()
        .label("Sincronizar")
        .css_classes(vec!["suggested-action"])
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();
    row_mtp.add_suffix(&btn_sync);
    group_mtp.add(&row_mtp);

    // Detectar dispositivos MTP montados
    {
        let row_mtp2 = row_mtp.clone();
        let btn_sync2 = btn_sync.clone();
        glib::MainContext::default().spawn_local(async move {
            let mtp_paths = detect_mtp_devices();
            if !mtp_paths.is_empty() {
                row_mtp2.set_subtitle(&format!("Detectado: {}", mtp_paths[0]));
                btn_sync2.set_sensitive(true);
            }
        });
    }

    // ── OPDS inalámbrico ───────────────────────────────────────────────
    let group_opds = adw::PreferencesGroup::builder()
        .title("OPDS Inalámbrico")
        .description("Activa el servidor de red local para conectar lectores como KOReader o Calibre.")
        .build();

    let port: u16 = 7891;
    let ip = crate::qr::local_ip();
    let opds_url = format!("http://{}:{}/opds", ip, port);

    let sw_opds = adw::SwitchRow::builder()
        .title("Servidor OPDS")
        .subtitle(&format!("Se publicará en {opds_url}"))
        .build();

    // QR code (inicialmente oculto)
    let qr_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build();

    let qr_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .halign(gtk::Align::Center)
        .build();

    let qr_widget = crate::qr::make_widget(&opds_url);
    qr_widget.set_halign(gtk::Align::Center);

    let url_lbl = gtk::Label::builder()
        .label(&opds_url)
        .selectable(true)
        .css_classes(vec!["monospace", "caption"])
        .build();
    let ip_hint = gtk::Label::builder()
        .label("Apuntá la cámara de tu Boox al código QR desde NeoReader o KOReader.")
        .css_classes(vec!["dim-label", "caption"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(48)
        .build();

    qr_box.append(&qr_widget);
    qr_box.append(&url_lbl);
    qr_box.append(&ip_hint);
    qr_revealer.set_child(Some(&qr_box));

    // Toggle OPDS
    {
        let db2 = db.clone();
        let rev = qr_revealer.clone();
        sw_opds.connect_active_notify(move |sw| {
            if sw.is_active() {
                let db3 = db2.clone();
                let rev2 = rev.clone();
                glib::MainContext::default().spawn_local(async move {
                    match SyncSubsystem::start_opds_server(db3, port).await {
                        Ok(()) => {
                            rev2.set_reveal_child(true);
                        }
                        Err(e) => eprintln!("Error iniciando OPDS: {e}"),
                    }
                });
            } else {
                rev.set_reveal_child(false);
                // El servidor tokio no tiene un shutdown handle aquí;
                // en producción se añadiría un CancellationToken.
            }
        });
    }

    group_opds.add(&sw_opds);

    // QR en un grupo sin título debajo del switch
    let qr_group = adw::PreferencesGroup::new();
    qr_group.add(&qr_revealer);

    page.add(&group_mtp);
    page.add(&group_opds);
    page.add(&qr_group);

    page
}

fn detect_mtp_devices() -> Vec<String> {
    let candidates = [
        "/run/user/1000/gvfs",
        "/media",
    ];
    let mut found = Vec::new();
    for base in &candidates {
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.contains("mtp") || name_str.contains("boox") || name_str.contains("onyx") {
                    found.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }
    found
}
