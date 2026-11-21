use crate::{
    i18n::tr,
    package::{get_deb_info, installed_desktop_entries},
};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

pub(crate) enum InstallationMessage {
    Started(bool),
    Output(String),
    Success(Vec<PathBuf>),
    Failed(String),
}

pub(crate) const INSTALL_OPTIONS: &[&str] = &["install", "-y", "--reinstall", "--allow-downgrades"];

pub(crate) fn start(
    path_string: String,
    removing: bool,
    tx_worker: mpsc::Sender<InstallationMessage>,
) {
    thread::spawn(move || {
        let _ = tx_worker.send(InstallationMessage::Started(removing));

        let package = get_deb_info(&path_string).map(|info| info.package);
        let args = if removing {
            let Some(info) = get_deb_info(&path_string) else {
                let _ = tx_worker.send(InstallationMessage::Failed(tr("Error reading package")));
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
            let _ = tx_worker.send(InstallationMessage::Failed(tr("Unable to start pkexec.")));
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
