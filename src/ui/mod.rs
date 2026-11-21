use crate::{
    dependencies::DependencyView,
    i18n::{GETTEXT_DOMAIN, tr},
};
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
mod actions;
mod progress;
mod selection;

fn load_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(include_str!("../../data/style.css"));
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::IconTheme::for_display(&display).add_search_path("data/icons");
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

pub(crate) fn build_ui(app: &libadwaita::Application, initial_path: Option<String>) {
    load_css();
    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title(tr("Debian Package Installer"))
        .default_width(560)
        .default_height(620)
        .build();

    let selected_file_path: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let template = gtk4::Builder::new();
    template.set_translation_domain(Some(GETTEXT_DOMAIN));
    template
        .add_from_string(include_str!(concat!(env!("OUT_DIR"), "/window.ui")))
        .expect("compiled Blueprint must be valid GTK XML");
    let toolbar = template
        .object::<libadwaita::ToolbarView>("toolbar")
        .expect("window template must contain toolbar");
    let menu_model = template
        .object::<gtk4::gio::MenuModel>("app_menu")
        .expect("template must define app_menu");
    let menu_button = template
        .object::<gtk4::MenuButton>("menu_button")
        .expect("template must define menu_button");
    menu_button.set_menu_model(Some(&menu_model));
    // --- GROUP 1: FILE SELECTION ---
    let file_row = template
        .object::<libadwaita::ActionRow>("file_row")
        .unwrap();
    let select_button = template.object::<gtk4::Button>("select_button").unwrap();
    let empty_page = template
        .object::<libadwaita::StatusPage>("empty_page")
        .unwrap();

    // --- GROUP 2: PACKAGE METADATA ---
    let info_group = template
        .object::<libadwaita::PreferencesGroup>("info_group")
        .unwrap();
    let package_name = template.object::<gtk4::Label>("package_name").unwrap();
    let package_version_row = template
        .object::<libadwaita::ActionRow>("package_version_row")
        .unwrap();
    package_version_row.set_title(&glib::markup_escape_text(&package_version_row.title()));
    package_version_row.set_use_markup(true);

    let installed_size_row = template
        .object::<libadwaita::ActionRow>("installed_size_row")
        .unwrap();
    let desc_row = template
        .object::<libadwaita::ActionRow>("desc_row")
        .unwrap();
    info_group.set_visible(false);
    let dependencies = DependencyView::new(&template);

    // --- INSTALL ACTION IN THE ADW HEADER BAR ---
    let install_button = template.object::<gtk4::Button>("install_button").unwrap();
    let uninstall_button = template.object::<gtk4::Button>("uninstall_button").unwrap();

    let controls_box = template.object::<gtk4::Box>("controls_box").unwrap();
    controls_box.set_visible(false);
    let progress_box = template.object::<gtk4::Box>("progress_box").unwrap();
    let progress_icon = template.object::<gtk4::Image>("progress_icon").unwrap();
    let progress_label = template.object::<gtk4::Label>("progress_label").unwrap();
    let progress_bar = template
        .object::<gtk4::ProgressBar>("progress_bar")
        .unwrap();
    let main_stack = template.object::<gtk4::Stack>("main_stack").unwrap();
    let back_button = template.object::<gtk4::Button>("back_button").unwrap();
    let footer = template.object::<libadwaita::Bin>("footer").unwrap();
    let progress_error = template.object::<gtk4::Label>("progress_error").unwrap();
    let launch_button = template.object::<gtk4::Button>("launch_button").unwrap();
    let launch_app: Rc<RefCell<Option<gio::DesktopAppInfo>>> = Rc::new(RefCell::new(None));
    let package_launch_button = template
        .object::<gtk4::Button>("package_launch_button")
        .unwrap();
    let launch_action = actions::connect_launch(
        window.clone(),
        launch_app.clone(),
        launch_button.clone(),
        package_launch_button.clone(),
    );
    main_stack.set_visible_child_name("package");
    let advanced_check = template
        .object::<gtk4::ToggleButton>("advanced_check")
        .unwrap();
    actions::connect_menu(
        template.clone(),
        advanced_check.clone(),
        window.clone(),
        select_button.clone(),
        controls_box.clone(),
        main_stack.clone(),
        launch_action.clone(),
        package_launch_button.clone(),
        launch_button.clone(),
        launch_app.clone(),
        install_button.clone(),
        uninstall_button.clone(),
        back_button.clone(),
    );
    let console = template.object::<gtk4::TextView>("console").unwrap();
    let console_buffer = console.buffer();
    let console_scroll = template
        .object::<gtk4::ScrolledWindow>("console_scroll")
        .unwrap();
    window.add_action_entries([gtk4::gio::ActionEntry::builder("about")
        .activate({
            let window = window.clone();
            move |_, _, _| {
                let dialog = libadwaita::AboutDialog::builder()
                    .application_name("Debby")
                    .application_icon("com.cobisimo.Debby")
                    .version(env!("CARGO_PKG_VERSION"))
                    .developer_name(tr("Debby contributors"))
                    .license_type(gtk4::License::Gpl30)
                    .comments(tr("Debby makes installing local Debian packages simple. Open a .deb file to view package details, compare installed and selected versions, and install, upgrade, reinstall, or uninstall the package. Follow each operation on a dedicated progress screen and view dependencies and command output in the Details panel."))
                    .build();
                dialog.present(Some(&window));
            }
        })
        .build()]);
    console_scroll.set_visible(false);

    let open_action = gtk4::gio::SimpleAction::new("open", None);
    open_action.connect_activate({
        let select_button = select_button.clone();
        move |action, _| {
            if action.is_enabled() && select_button.is_sensitive() {
                select_button.emit_clicked();
            }
        }
    });
    window.add_action(&open_action);
    select_button.connect_sensitive_notify({
        let action = open_action.clone();
        move |button| action.set_enabled(button.is_sensitive())
    });
    app.set_accels_for_action("win.open", &["<Control>o"]);
    let chooser_stack = main_stack.clone();
    let chooser_back = back_button.clone();
    let chooser_footer = footer.clone();

    let console_scroll_ui = console_scroll.clone();
    advanced_check.connect_toggled(move |check| {
        console_scroll_ui.set_visible(check.is_active());
    });

    selection::connect_back(
        back_button.clone(),
        main_stack.clone(),
        footer.clone(),
        selected_file_path.clone(),
        empty_page.clone(),
        info_group.clone(),
        file_row.clone(),
        package_name.clone(),
        package_version_row.clone(),
        installed_size_row.clone(),
        desc_row.clone(),
        install_button.clone(),
        uninstall_button.clone(),
        console_buffer.clone(),
        controls_box.clone(),
        package_launch_button.clone(),
        launch_app.clone(),
        dependencies.clone(),
    );
    let tx = progress::connect_progress(
        install_button.clone(),
        select_button.clone(),
        file_row.clone(),
        console_buffer.clone(),
        console_scroll.clone(),
        progress_box.clone(),
        progress_icon.clone(),
        progress_label.clone(),
        progress_bar.clone(),
        info_group.clone(),
        launch_app.clone(),
        package_launch_button.clone(),
        controls_box.clone(),
        launch_button.clone(),
        open_action.clone(),
        main_stack.clone(),
        back_button.clone(),
        footer.clone(),
        progress_error.clone(),
    );
    selection::connect_chooser(
        select_button.clone(),
        window.clone(),
        chooser_stack.clone(),
        chooser_back.clone(),
        chooser_footer.clone(),
        selected_file_path.clone(),
        empty_page.clone(),
        info_group.clone(),
        file_row.clone(),
        package_name.clone(),
        package_version_row.clone(),
        installed_size_row.clone(),
        desc_row.clone(),
        install_button.clone(),
        uninstall_button.clone(),
        console_buffer.clone(),
        controls_box.clone(),
        package_launch_button.clone(),
        launch_app.clone(),
        dependencies.clone(),
    );
    // --- ASYNCHRONOUS INSTALLATION ---
    for (button, removing) in [
        (install_button.clone(), false),
        (uninstall_button.clone(), true),
    ] {
        let file_path_install = selected_file_path.clone();
        let console_buffer_install = console_buffer.clone();
        let tx = tx.clone();
        let select = select_button.clone();
        let controls = controls_box.clone();
        button.connect_clicked(move |_| {
            if !select.is_sensitive() || !controls.is_visible() {
                return;
            }
            if let Some(ref path) = *file_path_install.borrow() {
                // Lock out repeat activations before the worker's Started message.
                select.set_sensitive(false);
                controls.set_visible(false);
                let path_string = path.clone();
                let tx_worker = tx.clone();
                let console_buffer = console_buffer_install.clone();
                console_buffer.set_text(&format!("{}\n", tr("Starting installation…")));

                crate::operations::start(path_string, removing, tx_worker);
            }
        });
    }
    window.set_content(Some(&toolbar));
    window.present();

    if let Some(path) = initial_path {
        selection::load_deb_file(
            Path::new(&path),
            &empty_page,
            &info_group,
            &file_row,
            &package_name,
            &package_version_row,
            &installed_size_row,
            &desc_row,
            &selected_file_path,
            &install_button,
            &uninstall_button,
            &console_buffer,
            &controls_box,
            &package_launch_button,
            &launch_app,
            &dependencies,
        );
    }
}
