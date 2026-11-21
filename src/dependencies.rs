use crate::{i18n::tr, operations::INSTALL_OPTIONS, package_icon};
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita::prelude::PreferencesRowExt;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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
pub(crate) struct DependencyView {
    group: libadwaita::PreferencesGroup,
    rows: gtk4::ListBox,
    generation: Rc<Cell<u64>>,
    stack: gtk4::Stack,
    details: gtk4::Box,
    pending: Rc<RefCell<Option<(PathBuf, String)>>>,
    icon: gtk4::Image,
}

impl DependencyView {
    pub(crate) fn new(template: &gtk4::Builder) -> Self {
        let view = Self {
            group: template.object("dependencies_group").unwrap(),
            rows: template.object("dependency_rows").unwrap(),
            generation: Rc::new(Cell::new(0)),
            stack: template.object("package_stack").unwrap(),
            details: template.object("package_details").unwrap(),
            pending: Rc::new(RefCell::new(None)),
            icon: template.object("package_icon").unwrap(),
        };
        view.stack.connect_visible_child_name_notify({
            let view = view.clone();
            move |_| view.load_pending()
        });
        view
    }

    pub(crate) fn set_package(&self, path: PathBuf, package: String) {
        self.load_icon(path.clone(), package.clone());
        *self.pending.borrow_mut() = Some((path, package));
        self.details.set_visible(true);
    }

    pub(crate) fn reset(&self) {
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
