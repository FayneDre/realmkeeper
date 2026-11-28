//! Whether a folder can hold a data root (ADR 0020): not inside a folder a
//! sync client uploads (a MySQL data directory synchronised while the server
//! writes it ends up corrupted), and on a file system that supports the
//! directory links of the server data overlay (NTFS; not FAT32 / exFAT or
//! most network shares).

use std::path::{Path, PathBuf};

use crate::{LauncherError, Result};

/// Folders known to be synchronised, with the client's name.
fn synced_folders() -> Vec<(PathBuf, &'static str)> {
    let mut folders = Vec::new();
    for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Some(path) = std::env::var_os(var).filter(|p| !p.is_empty()) {
            folders.push((PathBuf::from(path), "OneDrive"));
        }
    }
    // Dropbox writes its folders in `info.json` (personal, business).
    for base in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(base) else {
            continue;
        };
        let info = PathBuf::from(base).join("Dropbox").join("info.json");
        if let Ok(bytes) = std::fs::read(info) {
            folders.extend(dropbox_folders(&bytes).into_iter().map(|p| (p, "Dropbox")));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        folders.push((PathBuf::from(home).join("iCloudDrive"), "iCloud Drive"));
    }
    folders.extend(linux_synced_folders(true));
    folders
}

/// Folders of the Linux sync clients (ADR 0031): Syncthing and Nextcloud
/// folders, Dropbox ones (`all`: business accounts too). Read from their
/// configuration files under the home folder; empty on Windows.
fn linux_synced_folders(all: bool) -> Vec<(PathBuf, &'static str)> {
    let Some(home) = std::env::var_os("HOME").filter(|_| !cfg!(windows)) else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    let mut folders = Vec::new();
    for file in [
        state.join("syncthing/config.xml"),
        config.join("syncthing/config.xml"),
    ] {
        if let Ok(text) = std::fs::read_to_string(file) {
            folders.extend(
                syncthing_folders(&text, &home)
                    .into_iter()
                    .map(|p| (p, "Syncthing")),
            );
            break;
        }
    }
    if let Ok(text) = std::fs::read_to_string(config.join("Nextcloud/nextcloud.cfg")) {
        folders.extend(
            nextcloud_folders(&text)
                .into_iter()
                .map(|p| (p, "Nextcloud")),
        );
    }
    if let Ok(bytes) = std::fs::read(home.join(".dropbox/info.json")) {
        let paths = if all {
            dropbox_folders(&bytes)
        } else {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("personal")?.get("path")?.as_str().map(PathBuf::from))
                .into_iter()
                .collect()
        };
        folders.extend(paths.into_iter().map(|p| (p, "Dropbox")));
    }
    folders
}

/// `path` attributes of the `<folder>` elements of Syncthing's
/// `config.xml` (`~` is the home folder).
fn syncthing_folders(config: &str, home: &Path) -> Vec<PathBuf> {
    config
        .split("<folder ")
        .skip(1)
        .filter_map(|element| {
            let tag = element.split('>').next()?;
            let start = tag
                .find(" path=\"")
                .map(|i| i + 7)
                .or_else(|| tag.starts_with("path=\"").then_some(6))?;
            let value = &tag[start..];
            let value = &value[..value.find('"')?];
            let value = value
                .replace("&amp;", "&")
                .replace("&apos;", "'")
                .replace("&quot;", "\"");
            Some(match value.strip_prefix('~') {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None => PathBuf::from(value),
            })
        })
        .collect()
}

/// `localPath` of the synchronised folders in Nextcloud's `nextcloud.cfg`
/// (`0\Folders\1\localPath=/home/deck/Nextcloud/`).
fn nextcloud_folders(config: &str) -> Vec<PathBuf> {
    config
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.ends_with("localPath")
                .then(|| PathBuf::from(value.trim()))
        })
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// Synchronised folders of personal accounts (a handoff folder must not be
/// a work one, ADR 0028): personal OneDrive and Dropbox.
pub fn personal_synced_folders() -> Vec<(PathBuf, &'static str)> {
    let mut folders = Vec::new();
    if let Some(path) = std::env::var_os("OneDriveConsumer").filter(|p| !p.is_empty()) {
        folders.push((PathBuf::from(path), "OneDrive"));
    }
    for base in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(base) else {
            continue;
        };
        let info = PathBuf::from(base).join("Dropbox").join("info.json");
        if let Ok(bytes) = std::fs::read(info) {
            let personal = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("personal")?.get("path")?.as_str().map(PathBuf::from));
            if let Some(path) = personal {
                if !folders.iter().any(|(p, _)| p == &path) {
                    folders.push((path, "Dropbox"));
                }
            }
        }
    }
    folders.extend(linux_synced_folders(false));
    folders
}

fn dropbox_folders(info: &[u8]) -> Vec<PathBuf> {
    let Ok(serde_json::Value::Object(accounts)) = serde_json::from_slice(info) else {
        return Vec::new();
    };
    accounts
        .values()
        .filter_map(|account| account.get("path")?.as_str())
        .map(PathBuf::from)
        .collect()
}

/// Folder names Google Drive for desktop uses for its mirrored files.
const GOOGLE_DRIVE_NAMES: [&str; 3] = ["google drive", "my drive", "mon drive"];

/// The sync client that uploads `path`, if any.
pub fn synced_by(path: &Path) -> Option<&'static str> {
    synced_by_in(path, &synced_folders())
}

fn synced_by_in(path: &Path, folders: &[(PathBuf, &'static str)]) -> Option<&'static str> {
    let lower = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    let here = lower(path);
    for (folder, client) in folders {
        let folder = lower(folder);
        let folder = folder.trim_end_matches('\\');
        if folder.is_empty() {
            continue;
        }
        if here == folder || here.starts_with(&format!("{folder}\\")) {
            return Some(client);
        }
    }
    here.split('\\')
        .any(|name| GOOGLE_DRIVE_NAMES.contains(&name))
        .then_some("Google Drive")
}

/// Fails when `root` (an existing folder) cannot hold a data root.
pub fn check(root: &Path) -> Result<()> {
    if let Some(client) = synced_by(root) {
        return Err(LauncherError::LocationSynced {
            path: root.to_path_buf(),
            client: client.to_owned(),
        });
    }
    rk_engine::install::assets::supports_dir_links(root).map_err(|e| {
        LauncherError::LocationUnsupported {
            path: root.to_path_buf(),
            reason: e.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_under_a_sync_client_are_found() {
        let folders = vec![
            (PathBuf::from(r"C:\Users\p\OneDrive"), "OneDrive"),
            (PathBuf::from(r"D:\Dropbox\"), "Dropbox"),
        ];
        let synced = |p: &str| synced_by_in(Path::new(p), &folders);
        assert_eq!(synced(r"C:\Users\p\OneDrive\Games"), Some("OneDrive"));
        assert_eq!(synced(r"c:\users\p\onedrive"), Some("OneDrive"));
        assert_eq!(synced(r"C:\Users\p\OneDrive2\Games"), None);
        assert_eq!(synced(r"D:\Dropbox\WoW"), Some("Dropbox"));
        assert_eq!(synced(r"G:\My Drive\Realms"), Some("Google Drive"));
        assert_eq!(synced(r"D:\Games\Realms"), None);
    }

    #[test]
    fn linux_sync_clients_name_their_folders() {
        let home = Path::new("/home/deck");
        let syncthing = r#"<configuration version="37">
    <folder id="abcd-1234" label="RealmKeeper" path="~/Sync/RealmKeeper" type="sendreceive">
        <device id="X"></device>
    </folder>
    <folder id="efgh" label="Photos &amp; more" path="/run/media/deck/SD/Photos &amp; more"></folder>
    <device id="X" name="PC"></device>
</configuration>"#;
        assert_eq!(
            syncthing_folders(syncthing, home),
            vec![
                PathBuf::from("/home/deck/Sync/RealmKeeper"),
                PathBuf::from("/run/media/deck/SD/Photos & more")
            ]
        );
        let nextcloud = r"[Accounts]
0\Folders\1\localPath=/home/deck/Nextcloud/
0\url=https://cloud
";
        assert_eq!(
            nextcloud_folders(nextcloud),
            vec![PathBuf::from("/home/deck/Nextcloud/")]
        );
        let folders = vec![(PathBuf::from("/home/deck/Sync"), "Syncthing")];
        assert_eq!(
            synced_by_in(Path::new("/home/deck/Sync/Realms"), &folders),
            Some("Syncthing")
        );
        assert_eq!(
            synced_by_in(
                Path::new("/home/deck/.local/share/RealmKeeperData"),
                &folders
            ),
            None
        );
    }

    #[test]
    fn dropbox_info_lists_every_account() {
        let info = br#"{"personal":{"path":"C:\\Users\\p\\Dropbox","host":1},"business":{"path":"D:\\Work Dropbox"}}"#;
        let mut folders = dropbox_folders(info);
        folders.sort();
        assert_eq!(
            folders,
            vec![
                PathBuf::from(r"C:\Users\p\Dropbox"),
                PathBuf::from(r"D:\Work Dropbox")
            ]
        );
        assert!(dropbox_folders(b"not json").is_empty());
    }

    #[test]
    fn a_local_folder_holds_links() {
        let dir = tempfile::tempdir().unwrap();
        check(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "probe removed"
        );
    }
}
