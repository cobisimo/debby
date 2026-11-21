#![forbid(unsafe_code)]

use gtk4::prelude::*;

mod dependencies;
mod i18n;
mod operations;
mod package;
mod package_icon;
mod ui;

fn main() {
    i18n::setup_i18n();
    let application = libadwaita::Application::builder()
        .application_id("com.cobisimo.Debby")
        .flags(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let command_line_path = std::env::args().nth(1);
    application.connect_activate(move |app| ui::build_ui(app, command_line_path.clone()));
    application.connect_open(|app, files, _| {
        let path = files.first().and_then(|file| file.path());
        ui::build_ui(app, path.map(|path| path.to_string_lossy().to_string()));
    });
    application.run();
}
