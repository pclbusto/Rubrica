mod app;
mod cover;
mod qr;
mod pages;

use libadwaita as adw;
use libadwaita::prelude::*;
use tokio::runtime::Runtime;

fn main() {
    let rt = Runtime::new().expect("Error creando runtime Tokio");
    let _enter = rt.enter();

    let app = adw::Application::builder()
        .application_id("com.pedro.Rubrica")
        .build();

    app.connect_activate(app::build_ui);
    app.run();
}
