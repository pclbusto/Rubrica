use gtk::prelude::*;
use gtk::glib;
use gtk::{self, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use rubrica::library_db::{LibraryDb, BookListItem};

pub fn build(db: LibraryDb, nav_view: adw::NavigationView) -> (gtk::Widget, std::rc::Rc<dyn Fn()>) {
    let outer = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();

    // Modelo de lista
    let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();

    let selection = gtk::SingleSelection::builder()
        .model(&store)
        .build();

    let col_view = gtk::ColumnView::builder()
        .model(&selection)
        .show_row_separators(true)
        .show_column_separators(true)
        .reorderable(false)
        .build();

    // Columna: Miniatura
    {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let picture = gtk::Picture::builder()
                .width_request(32)
                .height_request(48)
                .content_fit(gtk::ContentFit::Cover)
                .can_shrink(true)
                .build();
            item.set_child(Some(&picture));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let picture = item.child().unwrap().downcast::<gtk::Picture>().unwrap();
            let obj = item.item().unwrap().downcast::<glib::BoxedAnyObject>().unwrap();
            let book: std::cell::Ref<BookListItem> = obj.borrow();
            let path = book.current_path.clone();
            let href = book.cover_href.clone();
            let pic_weak = picture.downgrade();
            glib::MainContext::default().spawn_local(async move {
                if let Some(px) = crate::cover::load(path, href, 32, 48).await {
                    if let Some(p) = pic_weak.upgrade() {
                        let tex = gtk::gdk::Texture::for_pixbuf(&px);
                        p.set_paintable(Some(tex.upcast_ref::<gtk::gdk::Paintable>()));
                    }
                }
            });
        });
        let col = gtk::ColumnViewColumn::builder()
            .title("Portada")
            .factory(&factory)
            .fixed_width(48)
            .resizable(false)
            .build();
        col_view.append_column(&col);
    }

    // Columna: Título + Autor
    {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let vbox = gtk::Box::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .margin_top(6)
                .margin_bottom(6)
                .build();
            let title_lbl = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(vec!["heading"])
                .build();
            title_lbl.set_widget_name("title");
            let author_lbl = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(vec!["dim-label", "caption"])
                .build();
            author_lbl.set_widget_name("author");
            vbox.append(&title_lbl);
            vbox.append(&author_lbl);
            item.set_child(Some(&vbox));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let vbox = item.child().unwrap().downcast::<gtk::Box>().unwrap();
            let obj = item.item().unwrap().downcast::<glib::BoxedAnyObject>().unwrap();
            let book: std::cell::Ref<BookListItem> = obj.borrow();
            let title_lbl = find_label_by_name(&vbox, "title");
            let author_lbl = find_label_by_name(&vbox, "author");
            if let Some(l) = title_lbl { l.set_text(&book.title); }
            if let Some(l) = author_lbl { l.set_text(&book.author_name); }
        });
        let col = gtk::ColumnViewColumn::builder()
            .title("Título / Autor")
            .factory(&factory)
            .expand(true)
            .resizable(true)
            .build();
        col_view.append_column(&col);
    }

    // Columna: Saga / Colección
    {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let lbl = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            item.set_child(Some(&lbl));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let lbl = item.child().unwrap().downcast::<gtk::Label>().unwrap();
            let obj = item.item().unwrap().downcast::<glib::BoxedAnyObject>().unwrap();
            let book: std::cell::Ref<BookListItem> = obj.borrow();
            lbl.set_text(book.series_name.as_deref().unwrap_or("—"));
        });
        let col = gtk::ColumnViewColumn::builder()
            .title("Saga")
            .factory(&factory)
            .fixed_width(180)
            .resizable(true)
            .build();
        col_view.append_column(&col);
    }

    // Columna: Salud del archivo
    {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let lbl = gtk::Label::builder()
                .xalign(0.0)
                .build();
            item.set_child(Some(&lbl));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let lbl = item.child().unwrap().downcast::<gtk::Label>().unwrap();
            let obj = item.item().unwrap().downcast::<glib::BoxedAnyObject>().unwrap();
            let book: std::cell::Ref<BookListItem> = obj.borrow();
            // Revisión rápida: el archivo existe en disco
            if std::path::Path::new(&book.current_path).exists() {
                lbl.set_text("✓ OK");
                lbl.remove_css_class("error");
                lbl.add_css_class("success");
            } else {
                lbl.set_text("✗ No encontrado");
                lbl.add_css_class("error");
            }
        });
        let col = gtk::ColumnViewColumn::builder()
            .title("Salud")
            .factory(&factory)
            .fixed_width(120)
            .resizable(false)
            .build();
        col_view.append_column(&col);
    }

    // Columna: Ubicación (normalizado o no)
    {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let img = gtk::Image::new();
            item.set_child(Some(&img));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let img = item.child().unwrap().downcast::<gtk::Image>().unwrap();
            let obj = item.item().unwrap().downcast::<glib::BoxedAnyObject>().unwrap();
            let book: std::cell::Ref<BookListItem> = obj.borrow();
            if book.is_normalized {
                img.set_from_icon_name(Some("folder-symbolic"));
                img.set_tooltip_text(Some("Archivo normalizado en Rúbrica"));
            } else {
                img.set_from_icon_name(Some("emblem-symbolic-link-symbolic"));
                img.set_tooltip_text(Some("Archivo en ubicación original"));
            }
        });
        let col = gtk::ColumnViewColumn::builder()
            .title("Ubicación")
            .factory(&factory)
            .fixed_width(80)
            .resizable(false)
            .build();
        col_view.append_column(&col);
    }

    scrolled.set_child(Some(&col_view));
    outer.append(&scrolled);

    // Clic en fila → navegar a detalle
    {
        let db2 = db.clone();
        let nav2 = nav_view.clone();
        selection.connect_selection_changed(move |sel, _, _| {
            if let Some(obj) = sel.selected_item() {
                let boxed = obj.downcast::<glib::BoxedAnyObject>().unwrap();
                let book: std::cell::Ref<BookListItem> = boxed.borrow();
                let book_id = book.id;
                drop(book);
                let page = crate::pages::detail::build(db2.clone(), book_id, nav2.clone());
                nav2.push(&page);
                // Deseleccionar para permitir volver a hacer clic
                sel.set_selected(gtk::INVALID_LIST_POSITION);
            }
        });
    }

    // Handle de recarga compartido
    let reload: std::rc::Rc<dyn Fn()> = {
        let db = db.clone();
        let store = store.clone();
        std::rc::Rc::new(move || {
            let db = db.clone();
            let store = store.clone();
            glib::MainContext::default().spawn_local(async move {
                match db.list_books().await {
                    Ok(books) => {
                        store.remove_all();
                        for b in books {
                            store.append(&glib::BoxedAnyObject::new(b));
                        }
                    }
                    Err(e) => eprintln!("Error recargando lista: {e}"),
                }
            });
        })
    };

    // Carga inicial
    reload();

    (outer.upcast(), reload)
}

fn find_label_by_name(container: &gtk::Box, name: &str) -> Option<gtk::Label> {
    let mut child = container.first_child();
    while let Some(w) = child {
        if w.widget_name() == name {
            return w.downcast::<gtk::Label>().ok();
        }
        child = w.next_sibling();
    }
    None
}
