use crate::{dependencies::DependencyView, i18n::tr, package::*};
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

pub(super) fn connect_back(
    back_button: gtk4::Button,
    main_stack: gtk4::Stack,
    footer: libadwaita::Bin,
    selected_file_path: Rc<RefCell<Option<String>>>,
    empty_page: libadwaita::StatusPage,
    info_group: libadwaita::PreferencesGroup,
    file_row: libadwaita::ActionRow,
    package_name: gtk4::Label,
    package_version_row: libadwaita::ActionRow,
    installed_size_row: libadwaita::ActionRow,
    desc_row: libadwaita::ActionRow,
    install_button: gtk4::Button,
    uninstall_button: gtk4::Button,
    console_buffer: gtk4::TextBuffer,
    controls_box: gtk4::Box,
    package_launch_button: gtk4::Button,
    launch_app: Rc<RefCell<Option<gio::DesktopAppInfo>>>,
    dependencies: DependencyView,
) {
    back_button.connect_clicked({
        let main_stack = main_stack.clone();
        let footer = footer.clone();
        let selected_file_path = selected_file_path.clone();
        let empty_page = empty_page.clone();
        let info_group = info_group.clone();
        let file_row = file_row.clone();
        let package_name = package_name.clone();
        let package_version_row = package_version_row.clone();

        let installed_size_row = installed_size_row.clone();
        let desc_row = desc_row.clone();
        let install_button = install_button.clone();
        let uninstall_button = uninstall_button.clone();
        let console_buffer = console_buffer.clone();
        let controls_box = controls_box.clone();
        let package_launch_button = package_launch_button.clone();
        let launch_app = launch_app.clone();
        let dependencies = dependencies.clone();
        move |button| {
            let path = selected_file_path.borrow().clone();
            if let Some(path) = path {
                load_deb_file(
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
            main_stack.set_visible_child_name("package");
            footer.set_visible(true);
            button.set_visible(false);
        }
    });
}

pub(super) fn connect_chooser(
    select_button: gtk4::Button,
    window: libadwaita::ApplicationWindow,
    chooser_stack: gtk4::Stack,
    chooser_back: gtk4::Button,
    chooser_footer: libadwaita::Bin,
    selected_file_path: Rc<RefCell<Option<String>>>,
    empty_page: libadwaita::StatusPage,
    info_group: libadwaita::PreferencesGroup,
    file_row: libadwaita::ActionRow,
    package_name: gtk4::Label,
    package_version_row: libadwaita::ActionRow,
    installed_size_row: libadwaita::ActionRow,
    desc_row: libadwaita::ActionRow,
    install_button: gtk4::Button,
    uninstall_button: gtk4::Button,
    console_buffer: gtk4::TextBuffer,
    controls_box: gtk4::Box,
    package_launch_button: gtk4::Button,
    launch_app: Rc<RefCell<Option<gio::DesktopAppInfo>>>,
    dependencies: DependencyView,
) {
    // --- LOGIKA ZA DIJALOG IZBORA FAJLA ---
    let file_row_clone = file_row.clone();
    let package_name_clone = package_name.clone();
    let package_version_row_clone = package_version_row.clone();
    let installed_size_row_clone = installed_size_row.clone();
    let desc_row_clone = desc_row.clone();
    let install_button_clone = install_button.clone();
    let uninstall_button_clone = uninstall_button.clone();
    let file_path_clone = selected_file_path.clone();
    let empty_page_clone = empty_page.clone();
    let info_group_clone = info_group.clone();
    let console_buffer_clone = console_buffer.clone();
    let controls_box_clone = controls_box.clone();
    let package_launch_button_clone = package_launch_button.clone();
    let launch_app_clone = launch_app.clone();
    let window_clone = window.clone();
    let dependencies_clone = dependencies.clone();

    select_button.connect_clicked(move |_| {
        let file_dialog = gtk4::FileChooserNative::builder()
            .title(tr("Choose a .deb package"))
            .accept_label(tr("Open"))
            .cancel_label(tr("Cancel"))
            .action(gtk4::FileChooserAction::Open)
            .transient_for(&window_clone)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some(&tr("Debian packages (*.deb)")));
        // add_suffix zamenjeno sa add_pattern u starijim GTK filterima radi kompatibilnosti
        filter.add_pattern("*.deb");

        file_dialog.set_filter(&filter);

        let file_row = file_row_clone.clone();
        let empty_page = empty_page_clone.clone();
        let info_group = info_group_clone.clone();
        let package_name = package_name_clone.clone();
        let package_version_row = package_version_row_clone.clone();
        let installed_size_row = installed_size_row_clone.clone();
        let desc_row = desc_row_clone.clone();
        let install_button = install_button_clone.clone();
        let uninstall_button = uninstall_button_clone.clone();
        let file_path = file_path_clone.clone();
        let console_buffer = console_buffer_clone.clone();
        let controls_box = controls_box_clone.clone();
        let package_launch_button = package_launch_button_clone.clone();
        let launch_app = launch_app_clone.clone();
        let chooser_stack = chooser_stack.clone();
        let chooser_back = chooser_back.clone();
        let chooser_footer = chooser_footer.clone();
        let dependencies = dependencies_clone.clone();

        file_dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept {
                if let Some(file) = dialog.file() {
                    if let Some(path) = file.path() {
                        load_deb_file(
                            &path,
                            &empty_page,
                            &info_group,
                            &file_row,
                            &package_name,
                            &package_version_row,
                            &installed_size_row,
                            &desc_row,
                            &file_path,
                            &install_button,
                            &uninstall_button,
                            &console_buffer,
                            &controls_box,
                            &package_launch_button,
                            &launch_app,
                            &dependencies,
                        );
                        chooser_stack.set_visible_child_name("package");
                        chooser_back.set_visible(false);
                        chooser_footer.set_visible(true);
                    }
                }
            }
            dialog.destroy();
        });
        file_dialog.show();
    });
}

pub(super) fn load_deb_file(
    path: &Path,
    empty_page: &libadwaita::StatusPage,
    info_group: &libadwaita::PreferencesGroup,
    file_row: &libadwaita::ActionRow,
    package_name: &gtk4::Label,
    package_version_row: &libadwaita::ActionRow,

    installed_size_row: &libadwaita::ActionRow,
    desc_row: &libadwaita::ActionRow,
    selected_file_path: &Rc<RefCell<Option<String>>>,
    install_button: &gtk4::Button,
    uninstall_button: &gtk4::Button,
    console_buffer: &gtk4::TextBuffer,
    controls_box: &gtk4::Box,
    package_launch_button: &gtk4::Button,
    launch_app: &Rc<RefCell<Option<gio::DesktopAppInfo>>>,
    dependencies: &DependencyView,
) {
    dependencies.reset();
    package_launch_button.set_visible(false);
    install_button.remove_css_class("warning-action");
    install_button.add_css_class("suggested-action");
    launch_app.borrow_mut().take();
    let absolute_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = absolute_path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    file_row.set_subtitle(&file_name);

    if let Some(info) = get_deb_info(&path_str) {
        package_name.set_text(&info.package);
        let installed = installed_version(&info.package);
        uninstall_button.set_visible(installed.is_some());
        if installed.is_some() {
            let app = visible_desktop_app(&installed_desktop_entries(&info.package));
            let can_launch = app.is_some();
            *launch_app.borrow_mut() = app;
            package_launch_button.set_visible(can_launch);
        }
        package_version_row.set_subtitle(&version_subtitle(
            &info.version,
            installed.as_deref(),
            &tr("{version} (installed version is {installed})"),
        ));
        installed_size_row.set_subtitle(
            &info
                .installed_size_kib
                .map(format_installed_size)
                .unwrap_or_else(|| tr("Unknown")),
        );
        desc_row.set_subtitle(&info.description);
        *selected_file_path.borrow_mut() = Some(path_str);
        empty_page.set_visible(false);
        info_group.set_visible(true);
        controls_box.set_visible(true);
        let (label, icon_name, is_reinstall) = install_button_label(&info.package, &info.version);
        let content = install_button
            .child()
            .unwrap()
            .downcast::<gtk4::Box>()
            .unwrap();
        let image = content
            .first_child()
            .unwrap()
            .downcast::<gtk4::Image>()
            .unwrap();
        let text = image
            .next_sibling()
            .unwrap()
            .downcast::<gtk4::Label>()
            .unwrap();
        image.set_icon_name(Some(icon_name));
        text.set_label(&label);
        install_button.set_tooltip_text(Some(&label));
        install_button.update_property(&[gtk4::accessible::Property::Label(&label)]);
        if is_reinstall {
            install_button.remove_css_class("suggested-action");
            install_button.add_css_class("warning-action");
        }
        console_buffer.set_text(&format!(
            "{}: {}\n{}: {}\n",
            tr("Package name"),
            info.package,
            tr("Version"),
            info.version
        ));
        dependencies.set_package(absolute_path, info.package);
        install_button.set_sensitive(true);
    } else {
        selected_file_path.borrow_mut().take();
        uninstall_button.set_visible(false);
        empty_page.set_visible(true);
        info_group.set_visible(false);
        controls_box.set_visible(false);
        package_name.set_text(&tr("Error reading package"));
        package_version_row.set_subtitle("-");

        installed_size_row.set_subtitle("-");
        desc_row.set_subtitle("-");
        install_button.set_sensitive(false);
    }
}
