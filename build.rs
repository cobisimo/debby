use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=po");
    let locale_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("locale");
    for language in fs::read_to_string("po/LINGUAS")
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let directory = locale_dir.join(language).join("LC_MESSAGES");
        fs::create_dir_all(&directory).unwrap();
        let status = Command::new("msgfmt")
            .arg("--check")
            .arg(format!("po/{language}.po"))
            .arg("-o")
            .arg(directory.join("debby.mo"))
            .status()
            .expect("msgfmt (gettext) is required to build Debby");
        assert!(
            status.success(),
            "Invalid translation catalog for {language}"
        );
    }
    println!("cargo:rerun-if-env-changed=DEBBY_INSTALL_LOCALEDIR");
    let runtime_locale_dir = env::var("DEBBY_INSTALL_LOCALEDIR")
        .unwrap_or_else(|_| locale_dir.to_string_lossy().into_owned());
    println!("cargo:rustc-env=DEBBY_LOCALEDIR={runtime_locale_dir}");
    println!("cargo:rerun-if-changed=data/window.blp");

    let out_file = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set")).join("window.ui");
    let status = Command::new("blueprint-compiler")
        .args(["compile", "--output"])
        .arg(&out_file)
        .arg("data/window.blp")
        .status()
        .expect("blueprint-compiler is required to build Debby");

    assert!(
        status.success(),
        "blueprint-compiler failed to compile data/window.blp"
    );
}
