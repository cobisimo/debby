use gtk4::{gdk_pixbuf, gio, glib};
use std::{
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

const FILE_LIMIT: usize = 2 * 1024 * 1024;
const ARCHIVE_LIMIT: u64 = 512 * 1024 * 1024;

fn safe_path(path: &str) -> Option<&str> {
    let path = path.strip_prefix("./").unwrap_or(path);
    (!path.is_empty()
        && !path.starts_with('/')
        && path.split('/').all(|p| !matches!(p, "" | "." | "..")))
    .then_some(path)
}

fn desktop_icon(text: &str) -> Option<String> {
    let mut desktop = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            desktop = line == "[Desktop Entry]";
        }
        if desktop {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "Icon" && !value.trim().is_empty() {
                    return Some(value.trim().to_owned());
                }
            }
        }
    }
    None
}

fn image_path(path: &str) -> bool {
    [".png", ".svg", ".xpm"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

fn icon_path(path: &str) -> bool {
    (path.starts_with("usr/share/icons/") && path.contains("/apps/")
        || path.starts_with("usr/share/pixmaps/"))
        && [".png", ".svg", ".xpm"]
            .iter()
            .any(|ext| path.ends_with(ext))
}

fn matches_icon(path: &str, icon: &str) -> bool {
    if icon.contains('/') {
        return safe_path(icon.trim_start_matches('/')) == Some(path);
    }
    let name = path.rsplit('/').next().unwrap_or("");
    name == icon || name.rsplit_once('.').is_some_and(|(stem, _)| stem == icon)
}

fn octal(bytes: &[u8]) -> Option<u64> {
    u64::from_str_radix(
        std::str::from_utf8(bytes).ok()?.trim_matches(['\0', ' ']),
        8,
    )
    .ok()
}

// Read only ordinary ustar files. Links are never followed, and extension records
// and their affected entry are skipped rather than interpreting overridden paths.
fn scan(reader: &mut impl Read) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let mut retained = 0;
    let mut extended = false;
    loop {
        let mut header = [0u8; 512];
        reader.read_exact(&mut header)?;
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let checksum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    *b as u64
                }
            })
            .sum();
        if octal(&header[148..156]) != Some(checksum) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let size = octal(&header[124..136]).ok_or(io::ErrorKind::InvalidData)?;
        if size > ARCHIVE_LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let field = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default())
                .into_owned()
        };
        let name = field(&header[..100]);
        let prefix = field(&header[345..500]);
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = safe_path(&name);
        let desktop = path
            .is_some_and(|p| p.starts_with("usr/share/applications/") && p.ends_with(".desktop"));
        let keep = !extended
            && matches!(header[156], 0 | b'0')
            && path.is_some_and(|p| desktop || image_path(p))
            && size <= if desktop { 65536 } else { FILE_LIMIT as u64 }
            && files.len() < 128
            && retained + size <= 16 * 1024 * 1024;
        extended = matches!(header[156], b'x' | b'g' | b'L' | b'K');
        if header[156] == b'g' {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if keep {
            let mut bytes = vec![0; size as usize];
            reader.read_exact(&mut bytes)?;
            retained += size;
            files.push((path.unwrap().to_owned(), bytes));
        } else {
            let copied = io::copy(&mut reader.take(size), &mut io::sink())?;
            if copied != size {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        let padding = (512 - size % 512) % 512;
        if io::copy(&mut reader.take(padding), &mut io::sink())? != padding {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
    }
    Ok(files)
}

pub fn load(path: &Path, package: &str) -> Option<Vec<u8>> {
    let mut child = Command::new("dpkg-deb")
        .arg("--fsys-tarfile")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let child = Arc::new(Mutex::new(child));
    let watchdog = child.clone();
    let (done, wait) = mpsc::channel();
    let timer = thread::spawn(move || {
        if wait.recv_timeout(Duration::from_secs(15)).is_err() {
            let _ = watchdog.lock().unwrap().kill();
        }
    });
    let result = scan(&mut stdout.take(ARCHIVE_LIMIT));
    // Stop decompression even when the scan reaches its budget or the tar EOF.
    let _ = child.lock().unwrap().kill();
    let _ = child.lock().unwrap().wait();
    let _ = done.send(());
    let _ = timer.join();
    let mut files = result.ok()?;
    let icons: Vec<_> = files
        .iter()
        .filter(|(p, _)| p.ends_with(".desktop"))
        .filter_map(|(_, b)| desktop_icon(std::str::from_utf8(b).ok()?))
        .collect();
    files.retain(|(p, _)| icon_path(p) || icons.iter().any(|i| matches_icon(p, i)));
    files.sort_by_key(|(p, _)| {
        (
            !icons.iter().any(|i| matches_icon(p, i)),
            !matches_icon(p, package),
            !p.contains("/hicolor/"),
            !p.contains("/scalable/"),
            p.clone(),
        )
    });
    for (_, bytes) in files {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
        if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_stream_at_scale(
            &stream,
            128,
            128,
            true,
            gio::Cancellable::NONE,
        ) {
            if let Ok(png) = pixbuf.save_to_bufferv("png", &[]) {
                return Some(png);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_group_only() {
        assert_eq!(
            desktop_icon(
                "[Other]\nIcon=wrong\n[Desktop Entry]\nIcon=right\n[Desktop Action Test]\nIcon=wrong"
            ),
            Some("right".into())
        );
        assert_eq!(desktop_icon("[Other]\nIcon=wrong"), None);
    }
    #[test]
    fn paths_and_matching() {
        assert_eq!(
            safe_path("./usr/share/icons/a.png"),
            Some("usr/share/icons/a.png")
        );
        for p in ["/tmp/a", "../a", "usr/../a", "usr//a"] {
            assert!(safe_path(p).is_none());
        }
        assert!(matches_icon(
            "usr/share/icons/hicolor/scalable/apps/app.svg",
            "app"
        ));
        assert!(matches_icon(
            "usr/share/pixmaps/app.png",
            "/usr/share/pixmaps/app.png"
        ));
        assert!(!matches_icon("usr/share/pixmaps/other.png", "app"));
    }
    fn entry(archive: &mut Vec<u8>, name: &str, kind: u8, body: &[u8]) {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[124..136].copy_from_slice(format!("{:011o}\0", body.len()).as_bytes());
        header[156] = kind;
        header[148..156].fill(b' ');
        let sum: u64 = header.iter().map(|b| *b as u64).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        archive.extend_from_slice(&header);
        archive.extend_from_slice(body);
        archive.resize(archive.len().next_multiple_of(512), 0);
    }

    #[test]
    fn reads_regular_files_without_links_or_overridden_paths() {
        let mut archive = Vec::new();
        entry(
            &mut archive,
            "./usr/share/applications/app.desktop",
            b'0',
            b"[Desktop Entry]\nIcon=app",
        );
        entry(&mut archive, "usr/share/pixmaps/link.png", b'2', b"");
        entry(&mut archive, "../escape.png", b'0', b"bad");
        entry(&mut archive, "pax", b'x', b"override");
        entry(
            &mut archive,
            "usr/share/pixmaps/overridden.png",
            b'0',
            b"bad",
        );
        entry(&mut archive, "opt/app/icon.png", b'0', b"image");
        archive.extend_from_slice(&[0; 1024]);
        let files = scan(&mut archive.as_slice()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1], ("opt/app/icon.png".into(), b"image".to_vec()));
    }

    #[test]
    fn rejects_bad_archive() {
        assert!(scan(&mut &[1u8; 512][..]).is_err());
    }
}
