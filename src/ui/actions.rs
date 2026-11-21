use crate::i18n::tr;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub(super) fn connect_launch(
    window: libadwaita::ApplicationWindow,
    launch_app: Rc<RefCell<Option<gio::DesktopAppInfo>>>,
    launch_button: gtk4::Button,
    package_launch_button: gtk4::Button,
) -> gio::SimpleAction {
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

    launch_action
}

pub(super) fn connect_menu(
    template: gtk4::Builder,
    advanced_check: gtk4::ToggleButton,
    window: libadwaita::ApplicationWindow,
    select_button: gtk4::Button,
    controls_box: gtk4::Box,
    main_stack: gtk4::Stack,
    launch_action: gio::SimpleAction,
    package_launch_button: gtk4::Button,
    launch_button: gtk4::Button,
    launch_app: Rc<RefCell<Option<gio::DesktopAppInfo>>>,
    install_button: gtk4::Button,
    uninstall_button: gtk4::Button,
    back_button: gtk4::Button,
) {
    let package_menu = template.object::<gio::Menu>("package_menu").unwrap();
    let install_label = template.object::<gtk4::Label>("install_label").unwrap();
    install_label.connect_label_notify(move |label| {
        package_menu.remove(1);
        package_menu.insert(1, Some(&label.label()), Some("win.install"));
    });

    let mut button_actions = Vec::new();
    for (name, button) in [
        ("install", &install_button),
        ("uninstall", &uninstall_button),
        ("back", &back_button),
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate({
            let button = button.clone();
            move |action, _| {
                if action.is_enabled() && button.is_sensitive() && button.is_visible() {
                    button.emit_clicked();
                }
            }
        });
        window.add_action(&action);
        button_actions.push((action, button.clone()));
    }
    let details_action =
        gio::SimpleAction::new_stateful("details", None, &advanced_check.is_active().to_variant());
    details_action.connect_change_state({
        let check = advanced_check.clone();
        move |_, state| {
            if let Some(active) = state.and_then(|state| state.get::<bool>()) {
                check.set_active(active);
            }
        }
    });
    advanced_check.connect_toggled({
        let action = details_action.clone();
        move |check| action.set_state(&check.is_active().to_variant())
    });
    window.add_action(&details_action);

    // Observe ancestors explicitly: a child's visible property stays true when
    // its controls or stack page are hidden.
    let sync_actions: Rc<dyn Fn()> = Rc::new({
        let select = select_button.clone();
        let controls = controls_box.clone();
        let stack = main_stack.clone();
        let launch = launch_action.clone();
        let package_launch = package_launch_button.clone();
        let progress_launch = launch_button.clone();
        let launch_app = launch_app.clone();
        move || {
            let idle = select.is_sensitive();
            let package_page = stack.visible_child_name().as_deref() == Some("package");
            for (action, button) in &button_actions {
                let available = action.name() == "back" || (package_page && controls.is_visible());
                action
                    .set_enabled(idle && available && button.is_visible() && button.is_sensitive());
            }
            launch.set_enabled(
                idle && launch_app.borrow().is_some()
                    && if package_page {
                        controls.is_visible() && package_launch.is_visible()
                    } else {
                        progress_launch.is_visible()
                    },
            );
        }
    });
    for widget in [
        select_button.clone().upcast::<gtk4::Widget>(),
        controls_box.clone().upcast(),
        main_stack.clone().upcast(),
        install_button.clone().upcast(),
        uninstall_button.clone().upcast(),
        back_button.clone().upcast(),
        package_launch_button.clone().upcast(),
        launch_button.clone().upcast(),
    ] {
        for property in ["visible", "sensitive"] {
            let sync = sync_actions.clone();
            widget.connect_notify_local(Some(property), move |_, _| sync());
        }
    }
    main_stack.connect_visible_child_name_notify({
        let sync = sync_actions.clone();
        let launch_button = launch_button.clone();
        move |stack| {
            if stack.visible_child_name().as_deref() == Some("package") {
                launch_button.set_visible(false);
            }
            sync();
        }
    });
    sync_actions();
}
