use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::rc::Rc;

use rubrica::library_db::{LibraryDb, BookListItem};

/// Construye la página de grilla.
/// Devuelve el widget y un handle de recarga: llamarlo recarga la lista
/// respetando el filtro de búsqueda activo.
pub fn build(
    db: LibraryDb,
    nav_view: adw::NavigationView,
    search_entry: gtk::SearchEntry,
) -> (gtk::Widget, Rc<dyn Fn()>) {
    let outer = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let flowbox = gtk::FlowBox::builder()
        .valign(gtk::Align::Start)
        .max_children_per_line(12)
        .min_children_per_line(2)
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(12)
        .row_spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .homogeneous(true)
        .build();

    scrolled.set_child(Some(&flowbox));
    outer.append(&scrolled);

    // ── Función de recarga central ─────────────────────────────────────
    // Único lugar que sabe cómo cargar y poblar el FlowBox.
    // Lee el texto del SearchEntry para filtrar o no.
    let reload: Rc<dyn Fn()> = {
        let db = db.clone();
        let nav_view = nav_view.clone();
        let flowbox_weak = flowbox.downgrade();
        let entry_weak = search_entry.downgrade();

        Rc::new(move || {
            let db = db.clone();
            let nav_view = nav_view.clone();
            let flowbox_weak = flowbox_weak.clone();
            let query = entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .filter(|t| !t.trim().is_empty());

            glib::MainContext::default().spawn_local(async move {
                let books = if let Some(q) = query.as_deref() {
                    let fts_q = format!("{}*", q.replace('"', ""));
                    db.search_fts(&fts_q).await.unwrap_or_default()
                } else {
                    db.list_books().await.unwrap_or_default()
                };

                if let Some(fb) = flowbox_weak.upgrade() {
                    populate_flowbox(&fb, books, &db, &nav_view);
                }
            });
        })
    };

    // Búsqueda en tiempo real
    {
        let reload = reload.clone();
        search_entry.connect_search_changed(move |_| reload());
    }

    // Drag-and-drop para importar ePubs
    {
        let reload = reload.clone();
        let db = db.clone();
        let drop_target = gtk::DropTarget::new(
            gtk::gio::File::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        drop_target.connect_drop(move |_, value, _, _| {
            if let Ok(file) = value.get::<gtk::gio::File>() {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().to_string();
                    let db = db.clone();
                    let reload = reload.clone();
                    glib::MainContext::default().spawn_local(async move {
                        match rubrica::Pipeline::import_file(&db, path_str).await {
                            Ok(_) => reload(),
                            Err(e) => eprintln!("Error importando por drag-and-drop: {e}"),
                        }
                    });
                }
            }
            true
        });
        flowbox.add_controller(drop_target);
    }

    // Carga inicial
    reload();

    (outer.upcast(), reload)
}

/// Vacía el FlowBox y lo llena con las tarjetas de los libros dados.
fn populate_flowbox(
    flowbox: &gtk::FlowBox,
    books: Vec<BookListItem>,
    db: &LibraryDb,
    nav_view: &adw::NavigationView,
) {
    while let Some(child) = flowbox.first_child() {
        flowbox.remove(&child);
    }

    if books.is_empty() {
        let status = adw::StatusPage::builder()
            .title("Biblioteca vacía")
            .description("Arrastrá un ePub aquí o usá el botón + para importar.")
            .icon_name("book-open-symbolic")
            .build();
        flowbox.insert(&status, -1);
        return;
    }

    for book in books {
        let card = build_card(&book, db, nav_view);
        flowbox.insert(&card, -1);
    }
}

/// Tarjeta completa con portada, hover y navegación.
fn build_card(
    book: &BookListItem,
    db: &LibraryDb,
    nav_view: &adw::NavigationView,
) -> gtk::FlowBoxChild {
    let book_id = book.id;
    let (card, picture, hover_box) = build_card_inner(book);

    // Cargar portada async escalada al tamaño de la tarjeta
    {
        let path = book.current_path.clone();
        let href = book.cover_href.clone();
        let picture_weak = picture.downgrade();
        glib::MainContext::default().spawn_local(async move {
            if let Some(px) = crate::cover::load(path, href, 148, 210).await {
                if let Some(pic) = picture_weak.upgrade() {
                    let tex = gtk::gdk::Texture::for_pixbuf(&px);
                    pic.set_paintable(Some(tex.upcast_ref::<gtk::gdk::Paintable>()));
                }
            }
        });
    }

    // Conectar los 3 botones del hover (los tres hijos directos son Button)
    let children = hover_box.observe_children();
    let mut btn_iter = children
        .into_iter()
        .filter_map(|obj| obj.ok()?.downcast::<gtk::Button>().ok());

    if let Some(btn_read) = btn_iter.next() {
        let db = db.clone();
        let nav = nav_view.clone();
        btn_read.connect_clicked(move |_| {
            let page = crate::pages::reader::build(db.clone(), book_id, nav.clone());
            nav.push(&page);
        });
    }
    if let Some(btn_detail) = btn_iter.next() {
        let db = db.clone();
        let nav = nav_view.clone();
        btn_detail.connect_clicked(move |_| {
            let page = crate::pages::detail::build(db.clone(), book_id, nav.clone());
            nav.push(&page);
        });
    }
    if let Some(btn_delete) = btn_iter.next() {
        let db = db.clone();
        let card_weak = card.downgrade();
        btn_delete.connect_clicked(move |btn| {
            show_delete_dialog(btn, db.clone(), book_id, card_weak.clone());
        });
    }

    // Menú contextual (clic derecho)
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    {
        let db = db.clone();
        let nav = nav_view.clone();
        gesture.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let model = gtk::gio::Menu::new();
            model.append(Some("Ficha del libro"), None::<&str>);
            model.append(Some("Enviar a Onyx Boox"), None::<&str>);
            let popover = gtk::PopoverMenu::from_model(Some(&model));
            popover.set_parent(&gesture.widget());
            popover.popup();

            // clic en "Ficha"
            let db = db.clone();
            let nav = nav.clone();
            let _ = (db, nav); // will wire up via action groups later
        });
    }
    card.add_controller(gesture);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&card));
    child
}

/// Estructura visual de la tarjeta. Devuelve (card, picture, hover_buttons_box).
fn build_card_inner(book: &BookListItem) -> (gtk::Box, gtk::Picture, gtk::Box) {
    let card = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(vec!["card"])
        .width_request(148)
        .build();

    let overlay = gtk::Overlay::new();
    overlay.set_size_request(148, 210);
    overlay.set_overflow(gtk::Overflow::Hidden);

    // Picture con tamaño fijo — el pixbuf llega ya escalado desde cover::load
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .build();
    picture.set_size_request(148, 210);
    overlay.set_child(Some(&picture));

    // Botones flotantes sobre la imagen
    // set_measure_overlay(false) → no participan en el cálculo de tamaño
    // set_clip_overlay(true)     → quedan recortados al área del overlay
    let hover_box = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .spacing(8)
        .build();
    hover_box.set_visible(false);

    let btn_read = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text("Leer")
        .css_classes(vec!["circular", "osd"])
        .build();
    let btn_detail = gtk::Button::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Ficha")
        .css_classes(vec!["circular", "osd"])
        .build();
    let btn_delete = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Eliminar")
        .css_classes(vec!["circular", "osd", "destructive-action"])
        .build();

    hover_box.append(&btn_read);
    hover_box.append(&btn_detail);
    hover_box.append(&btn_delete);

    overlay.add_overlay(&hover_box);
    overlay.set_measure_overlay(&hover_box, false);
    overlay.set_clip_overlay(&hover_box, true);

    let mc = gtk::EventControllerMotion::new();
    let hb = hover_box.clone();
    mc.connect_enter(move |_, _, _| hb.set_visible(true));
    let hb2 = hover_box.clone();
    mc.connect_leave(move |_| hb2.set_visible(false));
    overlay.add_controller(mc);

    let title = gtk::Label::builder()
        .label(&book.title)
        .wrap(false)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .xalign(0.5)
        .max_width_chars(18)
        .css_classes(vec!["title-5"])
        .margin_top(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    let author = gtk::Label::builder()
        .label(&book.author_name)
        .wrap(false)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .xalign(0.5)
        .max_width_chars(18)
        .css_classes(vec!["dim-label", "caption"])
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    card.append(&overlay);
    card.append(&title);
    card.append(&author);

    (card, picture, hover_box)
}

fn show_delete_dialog(
    btn: &gtk::Button,
    db: LibraryDb,
    book_id: i64,
    card_weak: glib::WeakRef<gtk::Box>,
) {
    let parent = btn.root().and_downcast::<gtk::Window>();
    let dialog = gtk::AlertDialog::builder()
        .message("¿Eliminar libro?")
        .detail("El registro se eliminará de la biblioteca. El archivo físico no se borrará.")
        .buttons(["Cancelar", "Eliminar"])
        .cancel_button(0)
        .default_button(0)
        .build();

    dialog.choose(
        parent.as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            if result.ok() == Some(1) {
                let db = db.clone();
                let card_weak = card_weak.clone();
                glib::MainContext::default().spawn_local(async move {
                    if let Err(e) = db.delete_book(book_id, false).await {
                        eprintln!("Error eliminando libro: {e}");
                    } else if let Some(card) = card_weak.upgrade() {
                        if let Some(child) = card.parent() {
                            if let Some(fb) = child.parent() {
                                if let Some(flowbox) = fb.downcast_ref::<gtk::FlowBox>() {
                                    flowbox.remove(&child);
                                }
                            }
                        }
                    }
                });
            }
        },
    );
}
