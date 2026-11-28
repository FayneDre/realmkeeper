//! How the game client starts on this platform (ADR 0031, point 7).
//!
//! Windows runs the executable. Linux runs it through the newest Proton
//! installed with Steam (`proton run`, its own prefix under the data root),
//! else through `wine`. A Steam shortcut (Steam Input, overlay, Game Mode)
//! comes with the controller work (later ADR).
//!
//! Under Proton or Wine the child the launcher started is a wrapper that
//! may end before the game: "the game exited" is watched on the game's own
//! process, found in `/proc` by its name.

use std::path::{Path, PathBuf};

use crate::ClientError;

/// What starts the executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runner {
    /// The executable itself (Windows).
    Native,
    /// `<script> run <exe>` with `STEAM_COMPAT_DATA_PATH` /
    /// `STEAM_COMPAT_CLIENT_INSTALL_PATH`.
    Proton { script: PathBuf, steam: PathBuf },
    /// `wine <exe>` with `WINEPREFIX`.
    Wine(PathBuf),
}

impl Runner {
    /// Short name for logs and the diagnostic report.
    pub fn describe(&self) -> String {
        match self {
            Runner::Native => "native".to_owned(),
            Runner::Proton { script, .. } => script
                .parent()
                .and_then(|d| d.file_name())
                .map_or_else(|| "Proton".to_owned(), |n| n.to_string_lossy().into_owned()),
            Runner::Wine(path) => path.display().to_string(),
        }
    }
}

/// The runner of this computer: native on Windows; on Linux the newest
/// Proton, else `wine` from `PATH`.
pub fn runner() -> Result<Runner, ClientError> {
    if cfg!(windows) {
        return Ok(Runner::Native);
    }
    if let Some(found) = newest_proton() {
        return Ok(found);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join("wine"))
        .find(|wine| wine.is_file())
        .map(Runner::Wine)
        .ok_or(ClientError::NoRunner)
}

/// Steam installations of this user: native, the `~/.steam/steam` link
/// (SteamOS) and Flatpak; each listed once.
fn steam_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = Vec::new();
    for candidate in [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
    ] {
        if let Ok(real) = candidate.canonicalize() {
            if real.join("steamapps").is_dir() && !roots.contains(&real) {
                roots.push(real);
            }
        }
    }
    roots
}

/// Every installed Proton, with the Steam root it belongs to: official
/// ones in each library's `steamapps/common`, custom ones (GE-Proton...)
/// in `compatibilitytools.d`.
fn newest_proton() -> Option<Runner> {
    let mut found: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for root in steam_roots() {
        let mut libraries = vec![root.clone()];
        if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            for path in library_paths(&text) {
                if !libraries.contains(&path) {
                    libraries.push(path);
                }
            }
        }
        let mut dirs: Vec<PathBuf> = libraries
            .iter()
            .map(|l| l.join("steamapps/common"))
            .collect();
        dirs.push(root.join("compatibilitytools.d"));
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let script = entry.path().join("proton");
                if script.is_file() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    found.push((name, script, root.clone()));
                }
            }
        }
    }
    found
        .into_iter()
        .max_by(|a, b| {
            proton_rank(&a.0)
                .cmp(&proton_rank(&b.0))
                .then(b.1.cmp(&a.1))
        })
        .map(|(_, script, steam)| Runner::Proton { script, steam })
}

/// Order of preference of a Proton by its folder name: Valve's numbered
/// releases (newest first), then Experimental, then Hotfix, then custom
/// builds (GE-Proton, UMU-Proton: by their numbers).
fn proton_rank(name: &str) -> (u8, Vec<u32>) {
    let numbers: Vec<u32> = name
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    let lower = name.to_ascii_lowercase();
    let tier = if let Some(rest) = lower.strip_prefix("proton ") {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            3
        } else if rest.contains("experimental") {
            2
        } else {
            1
        }
    } else {
        0
    };
    (tier, numbers)
}

/// `"path"` values of Steam's `libraryfolders.vdf` (`\\` escapes).
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("\"path\"")?.trim();
            let value = rest.strip_prefix('"')?.strip_suffix('"')?;
            Some(PathBuf::from(value.replace("\\\\", "\\")))
        })
        .collect()
}

/// Whether a `/proc/<pid>` process is the game: Wine names it after the
/// Windows executable (`comm`, at most 15 bytes) and its command line ends
/// with that executable's path.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn is_game(comm: &str, cmdline: &[u8], executable: &str) -> bool {
    let name = executable.rsplit(['/', '\\']).next().unwrap_or(executable);
    let short: String = name.chars().take(15).collect();
    if !comm.trim().eq_ignore_ascii_case(&short) {
        return false;
    }
    let first = cmdline.split(|b| *b == 0).next().unwrap_or_default();
    String::from_utf8_lossy(first)
        .to_ascii_lowercase()
        .ends_with(&name.to_ascii_lowercase())
}

/// Whether a process of this user is still running the game (Linux).
#[cfg(target_os = "linux")]
pub(crate) fn game_running(executable: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let dir = entry.path();
        let is_pid = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
        is_pid
            && std::fs::read_to_string(dir.join("comm")).is_ok_and(|comm| {
                std::fs::read(dir.join("cmdline"))
                    .is_ok_and(|cmdline| is_game(&comm, &cmdline, executable))
            })
    })
}

/// Prefix directory of a runner under `base` (the client's folder in the
/// data root): Proton and Wine prefixes are kept apart, their layouts
/// differ.
pub(crate) fn prefix_dir(runner: &Runner, base: &Path) -> Option<PathBuf> {
    match runner {
        Runner::Native => None,
        Runner::Proton { .. } => Some(base.join("proton")),
        Runner::Wine(_) => Some(base.join("wine")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valve_numbered_proton_comes_first_then_experimental_then_custom() {
        let mut names = vec![
            "GE-Proton10-25",
            "Proton - Experimental",
            "Proton 9.0 (Beta)",
            "Proton 11.0",
            "Proton Hotfix",
            "Proton 7.0",
            "UMU-Proton-9.0-3",
        ];
        names.sort_by_key(|n| std::cmp::Reverse(proton_rank(n)));
        assert_eq!(
            names,
            [
                "Proton 11.0",
                "Proton 9.0 (Beta)",
                "Proton 7.0",
                "Proton - Experimental",
                "Proton Hotfix",
                "GE-Proton10-25",
                "UMU-Proton-9.0-3",
            ]
        );
    }

    #[test]
    fn library_folders_are_read_from_steam_vdf() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"/home/deck/.local/share/Steam"
		"label"		""
	}
	"1"
	{
		"path"		"/run/media/deck/SD"
	}
}"#;
        assert_eq!(
            library_paths(vdf),
            [
                PathBuf::from("/home/deck/.local/share/Steam"),
                PathBuf::from("/run/media/deck/SD")
            ]
        );
    }

    #[test]
    fn the_game_process_is_known_by_its_wine_name() {
        let exe = "/home/deck/Games/WoW 3.3.5a/Wow.exe";
        assert!(is_game(
            "Wow.exe\n",
            b"Z:\\home\\deck\\Games\\WoW 3.3.5a\\Wow.exe\0",
            exe
        ));
        assert!(is_game("Wow.exe", b"C:\\Games\\WOW.EXE\0-console\0", exe));
        assert!(!is_game("wineserver", b"/usr/bin/wineserver\0", exe));
        assert!(!is_game("Wow.exe", b"Z:\\other\\Launcher.exe\0", exe));
        // comm keeps 15 bytes of a longer name.
        assert!(is_game(
            "WowLongExecutab",
            b"Z:\\g\\WowLongExecutable.exe\0",
            "WowLongExecutable.exe"
        ));
    }

    #[test]
    fn proton_and_wine_prefixes_are_kept_apart() {
        let base = Path::new("/data/clients/wow-3-3-5a.prefix");
        let proton = Runner::Proton {
            script: "/s/Proton 11.0/proton".into(),
            steam: "/s".into(),
        };
        assert_eq!(prefix_dir(&proton, base), Some(base.join("proton")));
        assert_eq!(
            prefix_dir(&Runner::Wine("/usr/bin/wine".into()), base),
            Some(base.join("wine"))
        );
        assert_eq!(prefix_dir(&Runner::Native, base), None);
        assert_eq!(proton.describe(), "Proton 11.0");
    }
}
