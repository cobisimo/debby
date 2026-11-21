use crate::{i18n::tr, operations::InstallationMessage, package::visible_desktop_app};
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

pub(super) fn connect_progress(
    install_button: gtk4::Button,
    select_button: gtk4::Button,
    file_row: libadwaita::ActionRow,
    console_buffer: gtk4::TextBuffer,
    console_scroll: gtk4::ScrolledWindow,
    progress_box: gtk4::Box,
    progress_icon: gtk4::Image,
    progress_label: gtk4::Label,
    progress_bar: gtk4::ProgressBar,
    info_group: libadwaita::PreferencesGroup,
    launch_app: Rc<RefCell<Option<gio::DesktopAppInfo>>>,
    package_launch_button: gtk4::Button,
    controls_box: gtk4::Box,
    launch_button: gtk4::Button,
    open_action: gio::SimpleAction,
    main_stack: gtk4::Stack,
    back_button: gtk4::Button,
    footer: libadwaita::Bin,
    progress_error: gtk4::Label,
) -> mpsc::Sender<InstallationMessage> {
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
    let controls_box_ui = controls_box.clone();
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
                controls_box_ui.set_visible(false);
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
                let can_launch = app.is_some();
                *launch_app_ui.borrow_mut() = app;
                launch_button.set_visible(can_launch);
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

    tx
}
