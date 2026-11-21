use crate::i18n::tr;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Default, Clone)]
pub(crate) struct DebInfo {
    pub(crate) package: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) installed_size_kib: Option<u64>,
}

pub(crate) fn installed_desktop_entries(package: &str) -> Vec<PathBuf> {
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

pub(crate) fn visible_desktop_app(desktop_entries: &[PathBuf]) -> Option<gio::DesktopAppInfo> {
    desktop_entries.iter().find_map(|path| {
        let app = gio::DesktopAppInfo::from_filename(path)?;
        (app.should_show() && !app.is_hidden()).then_some(app)
    })
}

pub(crate) fn get_deb_info(path: &str) -> Option<DebInfo> {
    let output = Command::new("dpkg-deb").args(["-I", path]).output().ok()?;

    if !output.status.success() {
        return None;
    }

    Some(parse_deb_info(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_deb_info(stdout: &str) -> DebInfo {
    let mut info = DebInfo::default();
    let mut parsing_description = false;
    let mut description_lines = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();

        if line.starts_with(" Package:") {
            info.package = trimmed.strip_prefix("Package:").unwrap().trim().to_string();
            parsing_description = false;
        } else if line.starts_with(" Version:") {
            info.version = trimmed.strip_prefix("Version:").unwrap().trim().to_string();
            parsing_description = false;
        } else if line.starts_with(" Installed-Size:") {
            info.installed_size_kib = trimmed
                .strip_prefix("Installed-Size:")
                .and_then(parse_installed_size);
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
    info
}

fn parse_installed_size(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub(crate) fn format_installed_size(kib: u64) -> String {
    if kib < 1024 {
        format!("{kib} KiB")
    } else if kib < 1024 * 1024 {
        format!("{:.1} MiB", kib as f64 / 1024.0)
    } else {
        format!("{:.1} GiB", kib as f64 / (1024.0 * 1024.0))
    }
}

pub(crate) fn install_button_label(package: &str, version: &str) -> (String, &'static str, bool) {
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

pub(crate) fn installed_version(package: &str) -> Option<String> {
    let output = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}\t${Version}", package])
        .output()
        .ok()?;
    let result = String::from_utf8_lossy(&output.stdout);
    let (status, version) = result.trim().split_once('\t')?;
    (status == "install ok installed").then(|| version.to_string())
}

pub(crate) fn version_subtitle(version: &str, installed: Option<&str>, pattern: &str) -> String {
    let selected = format!("<b>{}</b>", glib::markup_escape_text(version));
    match installed {
        Some(installed) if installed != version => glib::markup_escape_text(pattern)
            .split("{version}")
            .map(|part| part.replace("{installed}", &glib::markup_escape_text(installed)))
            .collect::<Vec<_>>()
            .join(&selected),
        _ => selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_version_subtitle() {
        let pattern = "{version} (installed version is {installed})";
        assert_eq!(version_subtitle("2.0", None, pattern), "<b>2.0</b>");
        assert_eq!(version_subtitle("2.0", Some("2.0"), pattern), "<b>2.0</b>");
        assert_eq!(
            version_subtitle("2.0", Some("1.0"), pattern),
            "<b>2.0</b> (installed version is 1.0)"
        );
        assert_eq!(
            version_subtitle("v2", Some("v1"), pattern),
            "<b>v2</b> (installed version is v1)"
        );
        assert_eq!(
            version_subtitle(
                "2",
                Some("1"),
                "{version} (инсталирана верзија је {installed})"
            ),
            "<b>2</b> (инсталирана верзија је 1)"
        );
        assert_eq!(
            version_subtitle("{installed}", Some("{version}"), "{installed}: {version}"),
            "{version}: <b>{installed}</b>"
        );
    }

    #[test]
    fn package_version_subtitle_escapes_markup() {
        let version = "<new>&\"'";
        let escaped = "<b>&lt;new&gt;&amp;&quot;&apos;</b>";
        assert_eq!(version_subtitle(version, None, ""), escaped);
        assert_eq!(version_subtitle(version, Some(version), ""), escaped);
        assert_eq!(
            version_subtitle(
                version,
                Some("<old>&\"' {version}"),
                "<i>инсталирана & \"{installed}\"</i>: {version} / {version}"
            ),
            format!(
                "&lt;i&gt;инсталирана &amp; &quot;&lt;old&gt;&amp;&quot;&apos; {{version}}&quot;&lt;/i&gt;: {escaped} / {escaped}"
            )
        );
    }

    #[test]
    fn installed_size_metadata() {
        let info = parse_deb_info(
            " Package: example\n Version: 2.0\n Installed-Size: 1536\n Description: Example\n  Longer description\n",
        );
        assert_eq!(info.package, "example");
        assert_eq!(info.version, "2.0");
        assert_eq!(info.description, "Example Longer description");
        assert_eq!(info.installed_size_kib, Some(1536));
        assert_eq!(
            parse_deb_info(" Package: example\n").installed_size_kib,
            None
        );
        for value in ["", "-1", "+1", "1.5", "1024 KiB", "18446744073709551616"] {
            assert_eq!(parse_installed_size(value), None, "{value}");
            assert_eq!(
                parse_deb_info(&format!(" Installed-Size: {value}\n")).installed_size_kib,
                None
            );
        }
        assert_eq!(parse_installed_size(" 0 "), Some(0));
        assert_eq!(parse_installed_size("18446744073709551615"), Some(u64::MAX));
    }

    #[test]
    fn installed_size_binary_units() {
        for (kib, expected) in [
            (0, "0 KiB"),
            (1, "1 KiB"),
            (1023, "1023 KiB"),
            (1024, "1.0 MiB"),
            (1536, "1.5 MiB"),
            (1048576, "1.0 GiB"),
            (1572864, "1.5 GiB"),
        ] {
            assert_eq!(format_installed_size(kib), expected);
        }
    }
}
