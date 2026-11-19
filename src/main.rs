#![forbid(unsafe_code)]

use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::env;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

mod package_icon;

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
    Success(Vec<PathBuf>),
    Failed(String),
}

fn installed_desktop_entries(package: &str) -> Vec<PathBuf> {
    let Ok(output) = Command::new("dpkg-query")
        .args(["-L", "--", package])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.extension().is_some_and(|ext| ext == "desktop"))
        .collect()
}

fn visible_desktop_app(desktop_entries: &[PathBuf]) -> Option<gio::DesktopAppInfo> {
    desktop_entries.iter().find_map(|path| {
        let app = gio::DesktopAppInfo::from_filename(path)?;
        (app.should_show() && !app.is_hidden()).then_some(app)
    })
}

fn get_deb_dependencies(path: &Path) -> Result<String, String> {
    let output = Command::new("dpkg-deb")
        .arg("-f")
        .arg(path)
        .arg("Depends")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

const INSTALL_OPTIONS: &[&str] = &["install", "-y", "--reinstall", "--allow-downgrades"];

// An Inst line has an old version in brackets only for upgrades/reinstalls.
// Keep architecture qualifiers, and exclude the selected package itself.
fn newly_planned_packages(output: &str, selected: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in output.lines() {
        let Some(rest) = line.strip_prefix("Inst ") else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let Some(name) = fields.next() else { continue };
        if fields.next().is_some_and(|field| field.starts_with('('))
            && name.split(':').next() != Some(selected)
        {
            packages.push(name.to_string());
        }
    }
    packages.sort();
    packages.dedup();
    packages
}

fn dependency_matches(group: &str, planned: &str) -> bool {
    group.split('|').any(|alternative| {
        let name = alternative
            .trim()
            .split(|c: char| c.is_whitespace() || c == '(')
            .next()
            .unwrap_or_default();
        let (base, qualifier) = name.split_once(':').unwrap_or((name, ""));
        let (planned_base, architecture) = planned.split_once(':').unwrap_or((planned, ""));
        base == planned_base
            && (qualifier.is_empty()
                || qualifier == "any"
                || qualifier == "native"
                || architecture.is_empty()
                || qualifier == architecture)
    })
}

#[cfg(test)]
mod dependency_tests {
    use super::{dependency_matches, newly_planned_packages};

    #[test]
    fn plan_excludes_selected_upgrades_and_reinstalls() {
        let output = "Inst selected (1.0 local [amd64])\n\
                      Inst new-lib:amd64 (2.0 Debian [amd64])\n\
                      Inst upgraded [1.0] (2.0 Debian [amd64])\n\
                      Inst reinstalled [1.0] (1.0 Debian [amd64])\n\
                      Conf new-lib:amd64 (2.0 Debian [amd64])\n";
        assert_eq!(
            newly_planned_packages(output, "selected"),
            vec!["new-lib:amd64"]
        );
        assert!(newly_planned_packages("", "selected").is_empty());
    }

    #[test]
    fn matches_alternatives_without_confusing_constraints_or_names() {
        assert!(dependency_matches(
            "other (>= 1) | libfoo:any (<< 3)",
            "libfoo:amd64"
        ));
        assert!(dependency_matches("libfoo:amd64 (>= 2)", "libfoo:amd64"));
        assert!(!dependency_matches("libfoo:arm64", "libfoo:amd64"));
        assert!(!dependency_matches("libfoo-extra | other", "libfoo"));
        assert!(!dependency_matches("missing | other", "unrelated"));
    }
}

fn simulate_install(path: &Path, package: &str) -> Result<Vec<String>, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let output = Command::new("apt-get")
        .arg("--simulate")
        .args(INSTALL_OPTIONS)
        .arg(path)
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )
        .trim()
        .to_string());
    }
    Ok(newly_planned_packages(
        &String::from_utf8_lossy(&output.stdout),
        package,
    ))
}

#[derive(Clone)]
struct DependencyView {
    group: libadwaita::PreferencesGroup,
    rows: gtk4::ListBox,
    generation: Rc<Cell<u64>>,
    stack: gtk4::Stack,
    details: gtk4::Box,
    pending: Rc<RefCell<Option<(PathBuf, String)>>>,
    icon: gtk4::Image,
}

impl DependencyView {
    fn reset(&self) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        self.icon.set_icon_name(Some("package-x-generic"));
        self.pending.borrow_mut().take();
        self.stack.set_visible_child_name("information");
        self.details.set_visible(false);
    }

    fn load_icon(&self, path: PathBuf, package: String) {
        let generation = self.generation.get();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(package_icon::load(&path, &package));
        });
        let view = self.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if view.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Some(bytes)) => {
                    let stream =
                        gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
                    if let Ok(pixbuf) =
                        gtk4::gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE)
                    {
                        view.icon
                            .set_paintable(Some(&gtk4::gdk::Texture::for_pixbuf(&pixbuf)));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                _ => {}
            }
            glib::ControlFlow::Break
        });
    }

    fn load_pending(&self) {
        if self.stack.visible_child_name().as_deref() != Some("dependencies") {
            return;
        }
        let pending = self.pending.borrow_mut().take();
        if let Some((path, package)) = pending {
            self.load(path, package, self.generation.get());
        }
    }

    fn row(&self, title: &str, subtitle: &str) {
        let row = libadwaita::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .title_lines(0)
            .subtitle_lines(0)
            .build();
        row.set_use_markup(false);
        self.rows.append(&row);
    }

    fn load(&self, path: PathBuf, package: String, generation: u64) {
        self.group.set_visible(true);
        self.row(&tr("Checking dependencies…"), "");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let declared = get_deb_dependencies(&path);
            let plan = simulate_install(&path, &package);
            let _ = tx.send((declared, plan));
        });
        let view = self.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if view.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let (declared, plan) = match rx.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    view.row(&tr("Dependency information unavailable"), "");
                    return glib::ControlFlow::Break;
                }
            };
            while let Some(child) = view.rows.first_child() {
                view.rows.remove(&child);
            }
            match declared {
                Ok(declared) if declared.is_empty() => {
                    view.row(&tr("No declared dependencies."), "")
                }
                Ok(declared) => {
                    for group in declared
                        .split(',')
                        .map(str::trim)
                        .filter(|group| !group.is_empty())
                    {
                        let matches: Vec<_> = plan
                            .as_ref()
                            .ok()
                            .into_iter()
                            .flatten()
                            .filter(|name| dependency_matches(group, name))
                            .cloned()
                            .collect();
                        let status = if matches.is_empty() {
                            tr("Declared dependency (not an installed-status check)")
                        } else {
                            format!("{}: {}", tr("Will be installed"), matches.join(", "))
                        };
                        view.row(group, &status);
                    }
                }
                Err(error) => view.row(&tr("Dependency information unavailable"), &error),
            }
            match plan {
                Ok(packages) => {
                    if !packages.is_empty() {
                        view.row(&tr("Additional packages to be installed"),
                            &tr("APT preview, including indirect dependencies. The plan may change before installation."));
                        for package in packages {
                            view.row(&package, &tr("Will be installed"));
                        }
                    }
                }
                Err(error) => view.row(&tr("APT installation plan unavailable"), &error),
            }
            glib::ControlFlow::Break
        });
    }
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

fn install_button_label(package: &str, version: &str) -> (String, &'static str, bool) {
    let output = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}\t${Version}", package])
        .output();

    let Ok(output) = output else {
        return (tr("Install"), "system-software-install-symbolic", false);
    };

    let installed = String::from_utf8_lossy(&output.stdout);
    let Some((status, installed_version)) = installed.split_once('\t') else {
        return (tr("Install"), "system-software-install-symbolic", false);
    };

    if status != "install ok installed" {
        return (tr("Install"), "system-software-install-symbolic", false);
    }

    let is_upgrade = Command::new("dpkg")
        .args(["--compare-versions", version, "gt", installed_version])
        .status()
        .is_ok_and(|status| status.success());

    if is_upgrade {
        (tr("Upgrade"), "software-update-available-symbolic", false)
    } else {
        (tr("Reinstall"), "view-refresh-symbolic", true)
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
        name_row.set_subtitle(&info.package);
        let installed = installed_version(&info.package);
        uninstall_button.set_visible(installed.is_some());
        if installed.is_some() {
            let app = visible_desktop_app(&installed_desktop_entries(&info.package));
            package_launch_button.set_visible(app.is_some());
            *launch_app.borrow_mut() = app;
        }
        let installed = installed.unwrap_or_else(|| tr("Not installed"));
        version_row.set_subtitle(&format!("{}  →  {}", installed, info.version));
        desc_row.set_subtitle(&info.description);
        *selected_file_path.borrow_mut() = Some(path_str);
        empty_page.set_visible(false);
        info_group.set_visible(true);
        controls_box.set_visible(true);
        let (label, icon_name, is_reinstall) = install_button_label(&info.package, &info.version);
        let content = install_button
            .child()
            .expect("install button must have content");
        let icon = content
            .first_child()
            .and_downcast::<gtk4::Image>()
            .expect("install button content must start with an image");
        let text = content
            .last_child()
            .and_downcast::<gtk4::Label>()
            .expect("install button content must end with a label");
        text.set_label(&label);
        icon.set_icon_name(Some(icon_name));
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
        dependencies.load_icon(absolute_path.clone(), info.package.clone());
        *dependencies.pending.borrow_mut() = Some((absolute_path, info.package));
        dependencies.details.set_visible(true);
        install_button.set_sensitive(true);
    } else {
        selected_file_path.borrow_mut().take();
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
    let dependencies = DependencyView {
        group: template.object("dependencies_group").unwrap(),
        rows: template.object::<gtk4::ListBox>("dependency_rows").unwrap(),
        generation: Rc::new(Cell::new(0)),
        stack: template.object("package_stack").unwrap(),
        details: template.object("package_details").unwrap(),
        pending: Rc::new(RefCell::new(None)),
        icon: template.object("package_icon").unwrap(),
    };
    dependencies.stack.connect_visible_child_name_notify({
        let dependencies = dependencies.clone();
        move |_| dependencies.load_pending()
    });

    // --- AKCIJA ZA INSTALACIJU U ADW HEADER BARU ---
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
    let launch_action = gio::SimpleAction::new("launch", None);
    launch_action.connect_activate({
        let launch_app = launch_app.clone();
        let window = window.clone();
        move |_, _| {
            if let Some(app) = launch_app.borrow().as_ref() {
                let context = gtk4::prelude::WidgetExt::display(&window).app_launch_context();
                if let Err(error) = app.launch(&[], Some(&context)) {
                    let dialog = libadwaita::AlertDialog::builder()
                        .heading(tr("Unable to launch application"))
                        .body(error.to_string())
                        .build();
                    dialog.add_response("close", &tr("Close"));
                    dialog.set_close_response("close");
                    dialog.present(Some(&window));
                }
            }
        }
    });
    window.add_action(&launch_action);
    launch_button.set_action_name(Some("win.launch"));
    package_launch_button.set_action_name(Some("win.launch"));
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
                    &name_row,
                    &version_row,
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

    let launch_app_ui = launch_app.clone();
    let package_launch_button_ui = package_launch_button.clone();
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
                launch_button.set_visible(false);
                package_launch_button_ui.set_visible(false);
                launch_app_ui.borrow_mut().take();
                progress_bar_ui.set_visible(true);
                open_action.set_enabled(false);
                removing = uninstall;
                main_stack.set_visible_child_name("progress");
                back_button.set_visible(false);
                footer.set_visible(true);
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
            InstallationMessage::Success(desktop_entries) => {
                let app = if removing {
                    None
                } else {
                    visible_desktop_app(&desktop_entries)
                };
                launch_button.set_visible(app.is_some());
                *launch_app_ui.borrow_mut() = app;
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
                launch_button.set_visible(false);
                launch_app_ui.borrow_mut().take();
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
        let name_row = name_row_clone.clone();
        let version_row = version_row_clone.clone();
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
                            &name_row,
                            &version_row,
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

                    let package = get_deb_info(&path_string).map(|info| info.package);
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
                        let mut args = vec!["apt-get".to_string()];
                        args.extend(INSTALL_OPTIONS.iter().map(|option| option.to_string()));
                        args.push(path_string);
                        args
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
                            let desktop_entries = if removing {
                                Vec::new()
                            } else {
                                package
                                    .as_deref()
                                    .map(installed_desktop_entries)
                                    .unwrap_or_default()
                            };
                            let _ = tx_worker.send(InstallationMessage::Success(desktop_entries));
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
            &package_launch_button,
            &launch_app,
            &dependencies,
        );
    }
}
