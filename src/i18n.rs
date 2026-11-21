use gtk4::glib;
use std::env;

pub(crate) const GETTEXT_DOMAIN: &str = "debby";

pub(crate) fn tr(message: &str) -> String {
    glib::dgettext(Some(GETTEXT_DOMAIN), message).to_string()
}

pub(crate) fn setup_i18n() {
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
