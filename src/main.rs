#![forbid(unsafe_code)]

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::env;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const GETTEXT_DOMAIN: &str = "debby";

fn tr(message: &str) -> String {
    glib::dgettext(Some(GETTEXT_DOMAIN), message).to_string()
}

fn setup_i18n() {
    // Initialize the process locale before GTK or any worker threads.
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    let directory =
        env::var("DEBBY_LOCALEDIR").unwrap_or_else(|_| env!("DEBBY_LOCALEDIR").to_string());
    gettextrs::bindtextdomain(GETTEXT_DOMAIN, directory)
        .expect("failed to bind translation directory");
    gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8")
        .expect("failed to set translation encoding");
    gettextrs::textdomain(GETTEXT_DOMAIN).expect("failed to select translation domain");
}

fn load_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(include_str!("../data/style.css"));
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::IconTheme::for_display(&display).add_search_path("data/icons");
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[derive(Debug, Default, Clone)]
struct DebInfo {
    package: String,
    version: String,
    description: String,
}

enum InstallationMessage {
    Started(bool),
    Output(String),
    Success,
    Failed(String),
}

fn get_deb_dependencies(path: &Path) -> String {
    Command::new("dpkg-deb")
        .args(["-f", &path.to_string_lossy(), "Depends"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|dependencies| !dependencies.is_empty())
        .unwrap_or_else(|| tr("No declared dependencies."))
}

fn get_deb_info(path: &str) -> Option<DebInfo> {
    let output = Command::new("dpkg-deb").args(["-I", path]).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut info = DebInfo::default();
    let mut parsing_description = false;
    let mut description_lines = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();

        if line.starts_with(" Package:") {
            info.package = trimmed.strip_prefix("Package: ")?.to_string();
            parsing_description = false;
        } else if line.starts_with(" Version:") {
            info.version = trimmed.strip_prefix("Version: ")?.to_string();
            parsing_description = false;
        } else if line.starts_with(" Description:") {
            if let Some(desc) = trimmed.strip_prefix("Description: ") {
                description_lines.push(desc.to_string());
            }
            parsing_description = true;
        } else if parsing_description && line.starts_with("  ") {
            if trimmed == "." {
                description_lines.push("\n".to_string());
            } else {
                description_lines.push(trimmed.to_string());
            }
        } else if line.starts_with(" ") && !line.starts_with("  ") {
            parsing_description = false;
        }
    }

    info.description = description_lines.join(" ");
    Some(info)
}

fn install_button_label(package: &str, version: &str) -> String {
    let output = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}\t${Version}", package])
        .output();

    let Ok(output) = output else {
        return tr("Install Package");
    };

    let installed = String::from_utf8_lossy(&output.stdout);
    let Some((status, installed_version)) = installed.split_once('\t') else {
        return tr("Install Package");
    };

    if status != "install ok installed" {
        return tr("Install Package");
    }

    let is_upgrade = Command::new("dpkg")
        .args(["--compare-versions", version, "gt", installed_version])
        .status()
        .is_ok_and(|status| status.success());

    if is_upgrade {
        tr("Upgrade Package")
    } else {
        tr("Reinstall Package")
    }
}

fn installed_version(package: &str) -> Option<String> {
    let output = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}\t${Version}", package])
        .output()
        .ok()?;
    let result = String::from_utf8_lossy(&output.stdout);
    let (status, version) = result.trim().split_once('\t')?;
    (status == "install ok installed").then(|| version.to_string())
}

fn load_deb_file(
    path: &Path,
    empty_page: &libadwaita::StatusPage,
    info_group: &libadwaita::PreferencesGroup,
    file_row: &libadwaita::ActionRow,
    name_row: &libadwaita::ActionRow,
    version_row: &libadwaita::ActionRow,
    desc_row: &libadwaita::ActionRow,
    selected_file_path: &Rc<RefCell<Option<String>>>,
    install_button: &gtk4::Button,
    uninstall_button: &gtk4::Button,
    console_buffer: &gtk4::TextBuffer,
    controls_box: &gtk4::Box,
) {
    let path_str = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    file_row.set_subtitle(&file_name);

    if let Some(info) = get_deb_info(&path_str) {
        name_row.set_subtitle(&info.package);
        let installed = installed_version(&info.package);
        uninstall_button.set_visible(installed.is_some());
        let installed = installed.unwrap_or_else(|| tr("Not installed"));
        version_row.set_subtitle(&format!("{}  →  {}", installed, info.version));
        desc_row.set_subtitle(&info.description);
        *selected_file_path.borrow_mut() = Some(path_str);
        empty_page.set_visible(false);
        info_group.set_visible(true);
        controls_box.set_visible(true);
        install_button.set_label(&install_button_label(&info.package, &info.version));
        console_buffer.set_text(&format!(
            "{}: {}\n{}: {}\n\n{}:\n{}\n",
            tr("Package name"),
            info.package,
            tr("Version"),
            info.version,
            tr("Dependencies"),
            get_deb_dependencies(path)
        ));
        install_button.set_sensitive(true);
    } else {
        uninstall_button.set_visible(false);
        empty_page.set_visible(true);
        info_group.set_visible(false);
        controls_box.set_visible(false);
        name_row.set_subtitle(&tr("Error reading package"));
        version_row.set_subtitle("-");
        desc_row.set_subtitle("-");
        install_button.set_sensitive(false);
    }
}

fn main() {
    setup_i18n();
    let application = libadwaita::Application::builder()
        .application_id("com.cobisimo.Debby")
        .flags(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let command_line_path = std::env::args().nth(1);
    application.connect_activate(move |app| build_ui(app, command_line_path.clone()));
    application.connect_open(|app, files, _| {
        let path = files.first().and_then(|file| file.path());
        build_ui(app, path.map(|path| path.to_string_lossy().to_string()));
    });
    application.run();
}

fn build_ui(app: &libadwaita::Application, initial_path: Option<String>) {
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
    // --- GRUPA 1: IZBOR FAJLA ---
    let file_row = template
        .object::<libadwaita::ActionRow>("file_row")
        .unwrap();
    let select_button = template.object::<gtk4::Button>("select_button").unwrap();
    let empty_page = template
        .object::<libadwaita::StatusPage>("empty_page")
        .unwrap();

    // --- GRUPA 2: METAPODACI PAKETA ---
    let info_group = template
        .object::<libadwaita::PreferencesGroup>("info_group")
        .unwrap();
    let name_row = template
        .object::<libadwaita::ActionRow>("name_row")
        .unwrap();
    let version_row = template
        .object::<libadwaita::ActionRow>("version_row")
        .unwrap();
    let desc_row = template
        .object::<libadwaita::ActionRow>("desc_row")
        .unwrap();
    info_group.set_visible(false);

    // --- AKCIJA ZA INSTALACIJU U ADW HEADER BARU ---
    let install_button = template.object::<gtk4::Button>("install_button").unwrap();
    let uninstall_button = template.object::<gtk4::Button>("uninstall_button").unwrap();
    uninstall_button.set_label(&tr("Uninstall"));
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
    let footer = template.object::<gtk4::Box>("footer").unwrap();
    let progress_error = template.object::<gtk4::Label>("progress_error").unwrap();
    main_stack.set_visible_child_name("package");
    let advanced_check = template
        .object::<gtk4::ToggleButton>("advanced_check")
        .unwrap();
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
        move |_, _| select_button.emit_clicked()
    });
    window.add_action(&open_action);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    let chooser_stack = main_stack.clone();
    let chooser_back = back_button.clone();
    let chooser_footer = footer.clone();

    let console_scroll_ui = console_scroll.clone();
    advanced_check.connect_toggled(move |check| {
        console_scroll_ui.set_visible(check.is_active());
    });

    back_button.connect_clicked({
        let main_stack = main_stack.clone();
        let footer = footer.clone();
        let selected_file_path = selected_file_path.clone();
        let empty_page = empty_page.clone();
        let info_group = info_group.clone();
        let file_row = file_row.clone();
        let name_row = name_row.clone();
        let version_row = version_row.clone();
        let desc_row = desc_row.clone();
        let install_button = install_button.clone();
        let uninstall_button = uninstall_button.clone();
        let console_buffer = console_buffer.clone();
        let controls_box = controls_box.clone();
        move |button| {
            let path = selected_file_path.borrow().clone();
            if let Some(path) = path {
                load_deb_file(
                    Path::new(&path),
                    &empty_page,
                    &info_group,
                    &file_row,
                    &name_row,
                    &version_row,
                    &desc_row,
                    &selected_file_path,
                    &install_button,
                    &uninstall_button,
                    &console_buffer,
                    &controls_box,
                );
            }
            main_stack.set_visible_child_name("package");
            footer.set_visible(true);
            button.set_visible(false);
        }
    });

    // --- KANAL ZA ASINHRONU KOMUNIKACIJU ---
    // GLib 0.20 no longer exposes MainContext::channel, so poll a standard
    // channel from the GTK main loop instead.
    let (tx, rx) = mpsc::channel::<InstallationMessage>();

    let install_button_ui = install_button.clone();
    let select_button_ui = select_button.clone();
    let file_row_ui = file_row.clone();
    let console_buffer_ui = console_buffer.clone();
    let console_scroll_ui_output = console_scroll.clone();
    let progress_box_ui = progress_box.clone();
    let progress_icon_ui = progress_icon.clone();
    let progress_label_ui = progress_label.clone();
    let progress_bar_ui = progress_bar.clone();
    let info_group_ui = info_group.clone();
    let mut installation_running = false;
    let mut removing = false;

    glib::timeout_add_local(Duration::from_millis(50), move || {
        let msg = match rx.try_recv() {
            Ok(msg) => msg,
            Err(mpsc::TryRecvError::Empty) => {
                if installation_running {
                    progress_bar_ui.pulse();
                }
                return glib::ControlFlow::Continue;
            }
            Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        };
        match msg {
            InstallationMessage::Started(uninstall) => {
                progress_bar_ui.set_visible(true);
                open_action.set_enabled(false);
                removing = uninstall;
                main_stack.set_visible_child_name("progress");
                back_button.set_visible(false);
                footer.set_visible(false);
                advanced_check.set_active(false);
                progress_error.set_visible(false);
                progress_error.set_text("");
                console_buffer_ui.set_text(&format!(
                    "{}\n",
                    (if removing {
                        tr("Uninstalling…")
                    } else {
                        tr("Starting installation…")
                    })
                ));
                installation_running = true;
                progress_box_ui.set_visible(true);
                progress_icon_ui.set_visible(false);
                progress_label_ui.set_text(
                    &(if removing {
                        tr("Uninstalling…")
                    } else {
                        tr("Installation in progress…")
                    }),
                );
                progress_bar_ui.set_fraction(0.0);
                progress_bar_ui.set_text(Some(
                    &(if removing {
                        tr("Uninstalling…")
                    } else {
                        tr("Installing…")
                    }),
                ));
                info_group_ui.set_visible(false);
                install_button_ui.set_sensitive(false);
                select_button_ui.set_sensitive(false);
                file_row_ui.set_subtitle(&tr("Installation in progress…"));
            }
            InstallationMessage::Output(line) => {
                console_buffer_ui.insert_at_cursor(&line);
                console_buffer_ui.insert_at_cursor("\n");
                let adjustment = console_scroll_ui_output.vadjustment();
                let bottom = adjustment.upper() - adjustment.page_size();
                adjustment.set_value(bottom.max(adjustment.lower()));
            }
            InstallationMessage::Success => {
                progress_bar_ui.set_visible(false);
                open_action.set_enabled(true);
                back_button.set_visible(true);
                installation_running = false;
                progress_bar_ui.set_fraction(1.0);
                progress_bar_ui.set_text(Some(&tr("Complete")));
                progress_icon_ui.set_visible(true);
                progress_label_ui.set_text(
                    &(if removing {
                        tr("Uninstallation complete")
                    } else {
                        tr("Installation complete")
                    }),
                );
                info_group_ui.set_visible(false);
                select_button_ui.set_sensitive(true);
                install_button_ui.set_sensitive(true);
                file_row_ui.set_subtitle(&tr("Package installed successfully!"));
            }
            InstallationMessage::Failed(err_msg) => {
                progress_bar_ui.set_visible(false);
                open_action.set_enabled(true);
                back_button.set_visible(true);
                progress_error.set_text(&err_msg);
                progress_error.set_visible(true);
                installation_running = false;
                progress_icon_ui.set_visible(false);
                progress_label_ui.set_text(
                    &(if removing {
                        tr("Uninstallation failed")
                    } else {
                        tr("Installation failed")
                    }),
                );
                progress_bar_ui.set_fraction(0.0);
                progress_bar_ui.set_text(Some(&tr("Error")));
                info_group_ui.set_visible(false);
                install_button_ui.set_sensitive(true);
                select_button_ui.set_sensitive(true);
                file_row_ui.set_subtitle(&tr("Installation failed."));

                console_buffer_ui.insert_at_cursor(&format!("{}: {}\n", tr("Error"), err_msg));
            }
        }
        glib::ControlFlow::Continue
    });

    // --- LOGIKA ZA DIJALOG IZBORA FAJLA ---
    let file_row_clone = file_row.clone();
    let name_row_clone = name_row.clone();
    let version_row_clone = version_row.clone();
    let desc_row_clone = desc_row.clone();
    let install_button_clone = install_button.clone();
    let uninstall_button_clone = uninstall_button.clone();
    let file_path_clone = selected_file_path.clone();
    let empty_page_clone = empty_page.clone();
    let info_group_clone = info_group.clone();
    let console_buffer_clone = console_buffer.clone();
    let controls_box_clone = controls_box.clone();
    let window_clone = window.clone();

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
        let name_row = name_row_clone.clone();
        let version_row = version_row_clone.clone();
        let desc_row = desc_row_clone.clone();
        let install_button = install_button_clone.clone();
        let uninstall_button = uninstall_button_clone.clone();
        let file_path = file_path_clone.clone();
        let console_buffer = console_buffer_clone.clone();
        let controls_box = controls_box_clone.clone();
        let chooser_stack = chooser_stack.clone();
        let chooser_back = chooser_back.clone();
        let chooser_footer = chooser_footer.clone();

        file_dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept {
                if let Some(file) = dialog.file() {
                    if let Some(path) = file.path() {
                        load_deb_file(
                            &path,
                            &empty_page,
                            &info_group,
                            &file_row,
                            &name_row,
                            &version_row,
                            &desc_row,
                            &file_path,
                            &install_button,
                            &uninstall_button,
                            &console_buffer,
                            &controls_box,
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

    // --- ASINHRONA INSTALACIJA ---
    for (button, removing) in [
        (install_button.clone(), false),
        (uninstall_button.clone(), true),
    ] {
        let file_path_install = selected_file_path.clone();
        let console_buffer_install = console_buffer.clone();
        let tx = tx.clone();
        button.connect_clicked(move |_| {
            if let Some(ref path) = *file_path_install.borrow() {
                let path_string = path.clone();
                let tx_worker = tx.clone();
                let console_buffer = console_buffer_install.clone();
                console_buffer.set_text(&format!("{}\n", tr("Starting installation…")));

                thread::spawn(move || {
                    let _ = tx_worker.send(InstallationMessage::Started(removing));

                    let args = if removing {
                        let Some(info) = get_deb_info(&path_string) else {
                            let _ = tx_worker
                                .send(InstallationMessage::Failed(tr("Error reading package")));
                            return;
                        };
                        vec![
                            "apt-get".to_string(),
                            "remove".to_string(),
                            "-y".to_string(),
                            "--".to_string(),
                            info.package,
                        ]
                    } else {
                        vec![
                            "apt-get".to_string(),
                            "install".to_string(),
                            "-y".to_string(),
                            "--reinstall".to_string(),
                            "--allow-downgrades".to_string(),
                            path_string,
                        ]
                    };

                    let child = Command::new("pkexec")
                        .args(args)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn();

                    let Ok(mut child) = child else {
                        let _ = tx_worker
                            .send(InstallationMessage::Failed(tr("Unable to start pkexec.")));
                        return;
                    };

                    let stdout = child.stdout.take().expect("stdout was piped");
                    let stderr = child.stderr.take().expect("stderr was piped");
                    let tx_stdout = tx_worker.clone();
                    let tx_stderr = tx_worker.clone();
                    let stdout_thread = thread::spawn(move || {
                        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                            let _ = tx_stdout.send(InstallationMessage::Output(line));
                        }
                    });
                    let stderr_thread = thread::spawn(move || {
                        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                            let _ = tx_stderr.send(InstallationMessage::Output(line));
                        }
                    });

                    let status = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    match status {
                        Ok(status) if status.success() => {
                            let _ = tx_worker.send(InstallationMessage::Success);
                        }
                        Ok(_) => {
                            let _ = tx_worker.send(InstallationMessage::Failed(tr(
                                "Authorization was cancelled or the process failed.",
                            )));
                        }
                        Err(e) => {
                            let _ = tx_worker.send(InstallationMessage::Failed(e.to_string()));
                        }
                    }
                });
            }
        });
    }
    window.set_content(Some(&toolbar));
    window.present();

    if let Some(path) = initial_path {
        load_deb_file(
            Path::new(&path),
            &empty_page,
            &info_group,
            &file_row,
            &name_row,
            &version_row,
            &desc_row,
            &selected_file_path,
            &install_button,
            &uninstall_button,
            &console_buffer,
            &controls_box,
        );
    }
}
