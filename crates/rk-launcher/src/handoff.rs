//! Playing a realm on several computers (ADR 0028): its player data travels
//! through a folder the player already synchronises (Google Drive, OneDrive,
//! Dropbox...). The launcher only writes and reads files there.
//!
//! - `<app dir>/handoff.json`: this computer's id and its handoff folder.
//! - `instances/<id>/handoff.json` ([`Link`]): the realm's share id, the
//!   last generation sent or taken back, whether it was played since.
//! - `<folder>/<uid>/`: `state-<generation>-<machine>.rkrealm` (an export),
//!   `head-<machine>.json` ([`Head`], written last) and
//!   `playing-<machine>.json` while that computer's servers run. Each
//!   computer writes only its own names: no sync conflict copies.

use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use super::*;

const SETTINGS_FILE: &str = "handoff.json";
const LINK_FILE: &str = "handoff.json";
const FORMAT: u32 = 1;
/// Another computer's `playing-*.json` younger than this is worth a warning.
const PLAYING_FRESH_SECS: u64 = 12 * 3600;
/// Generations of the folder whose state files are kept.
const KEEP_GENERATIONS: usize = 2;

// ------------------------------------------------------------ settings

/// `<app dir>/handoff.json`: this computer's side of the handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Random id of this launcher (16 hex), in the folder's file names.
    pub machine_id: String,
    /// The handoff folder, none until the player picks one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
}

/// Where this computer hands realms off: the folder and who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub folder: PathBuf,
    pub machine_id: String,
    /// The computer's name, shown on the other side.
    pub machine: String,
}

fn random_hex(bytes: usize) -> Result<String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|e| LauncherError::Invalid(e.to_string()))?;
    Ok(hex::encode(buffer))
}

/// The computer's name (`COMPUTERNAME`), else a generic one.
pub fn machine_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.trim().is_empty()))
        .map(|v| v.trim().chars().take(64).collect())
        .unwrap_or_else(|| "PC".to_owned())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir.to_path_buf(), e))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(value).expect("serializable");
    std::fs::write(&tmp, json).map_err(|e| io_error(tmp.clone(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_error(path.to_path_buf(), e))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| LauncherError::Invalid(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(path.to_path_buf(), e)),
    }
}

/// This computer's settings; the machine id is made (and saved) the first
/// time.
pub fn settings(app_dir: &Path) -> Result<Settings> {
    let path = app_dir.join(SETTINGS_FILE);
    let read = read_json::<Settings>(&path).ok().flatten();
    match read {
        Some(settings) if settings.machine_id.len() == 16 => Ok(settings),
        _ => {
            let settings = Settings {
                machine_id: random_hex(8)?,
                folder: read.and_then(|s| s.folder),
            };
            write_json_atomic(&path, &settings)?;
            Ok(settings)
        }
    }
}

/// The place of this computer, `None` while no folder is chosen.
pub fn place(app_dir: &Path) -> Result<Option<Place>> {
    let settings = settings(app_dir)?;
    Ok(settings.folder.map(|folder| Place {
        folder,
        machine_id: settings.machine_id,
        machine: machine_name(),
    }))
}

/// Chooses the handoff folder (`None`: no handoff on this computer).
pub fn set_folder(app_dir: &Path, data_root: &Path, folder: Option<&Path>) -> Result<Settings> {
    if let Some(folder) = folder {
        check_folder(folder, data_root)?;
    }
    let mut settings = settings(app_dir)?;
    settings.folder = folder.map(Path::to_path_buf);
    write_json_atomic(&app_dir.join(SETTINGS_FILE), &settings)?;
    Ok(settings)
}

fn folder_error(folder: &Path, reason: &str) -> LauncherError {
    LauncherError::HandoffFolder {
        path: folder.to_path_buf(),
        reason: reason.to_owned(),
    }
}

fn normalised(path: &Path) -> String {
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    text.trim_end_matches('\\').to_owned()
}

fn inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (normalised(path), normalised(dir));
    path == dir || path.starts_with(&format!("{dir}\\"))
}

/// An existing, writable folder that neither holds nor sits in the data
/// root (its MySQL files must never be synchronised).
pub fn check_folder(folder: &Path, data_root: &Path) -> Result<()> {
    if !folder.is_absolute() || !folder.is_dir() {
        return Err(folder_error(folder, "not an existing folder"));
    }
    if inside(folder, data_root) || inside(data_root, folder) {
        return Err(folder_error(
            folder,
            "inside or around the launcher's data folder",
        ));
    }
    let probe = folder.join(format!(".rk-probe-{}", std::process::id()));
    std::fs::write(&probe, b"probe").map_err(|e| folder_error(folder, &e.to_string()))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// A folder the launcher can suggest, with the client syncing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub path: PathBuf,
    pub client: String,
}

/// `RealmKeeper` folders in the synchronised folders found on this
/// computer (Google Drive first: `X:\My Drive`, then personal OneDrive and
/// Dropbox). Work accounts' folders are left out.
pub fn suggested_folders() -> Vec<Suggestion> {
    let mut out = Vec::new();
    for letter in b'D'..=b'Z' {
        for name in ["My Drive", "Mon Drive"] {
            let path = PathBuf::from(format!("{}:\\{name}", letter as char));
            if path.is_dir() {
                out.push(Suggestion {
                    path: path.join("RealmKeeper"),
                    client: "Google Drive".into(),
                });
            }
        }
    }
    for (path, client) in location::personal_synced_folders() {
        if path.is_dir() {
            out.push(Suggestion {
                path: path.join("RealmKeeper"),
                client: client.into(),
            });
        }
    }
    out
}

// ------------------------------------------------------------ files

/// `instances/<id>/handoff.json`: a realm shared through the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub format: u32,
    /// Share id (32 hex): the realm's directory in the folder.
    pub uid: String,
    /// Last generation this computer sent or took back (0: none yet).
    pub generation: u64,
    /// The servers ran here since that generation.
    pub played: bool,
    /// Generation being taken back (a restore in progress).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulling: Option<u64>,
}

/// `head-<machine>.json`: the newest state one computer wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub format: u32,
    pub uid: String,
    pub generation: u64,
    pub parent: u64,
    pub machine_id: String,
    pub machine: String,
    /// Unix seconds.
    pub secs: u64,
    /// Emulator of the realm (catalog id).
    pub emulator: String,
    /// State file name in the realm's directory.
    pub file: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub build: Option<String>,
    pub launcher: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// `playing-<machine>.json`: that computer's servers run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playing {
    pub machine_id: String,
    pub machine: String,
    pub secs: u64,
}

fn valid_uid(uid: &str) -> bool {
    uid.len() == 32 && uid.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_machine(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn state_name(generation: u64, machine_id: &str) -> String {
    format!(
        "state-{generation}-{machine_id}.{}",
        realms::EXPORT_EXTENSION
    )
}

/// Generation and machine of a state file name.
fn parse_state_name(name: &str) -> Option<(u64, &str)> {
    let rest = name.strip_prefix("state-")?;
    let rest = rest.strip_suffix(&format!(".{}", realms::EXPORT_EXTENSION))?;
    let (generation, machine) = rest.split_once('-')?;
    valid_machine(machine).then_some(())?;
    Some((generation.parse().ok()?, machine))
}

/// A head read from the folder, with whether its state file has arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenHead {
    pub head: Head,
    /// The state file is there with the head's size.
    pub arrived: bool,
}

/// What the realm's directory holds: heads (newest first) and the other
/// computers' playing marks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderView {
    pub heads: Vec<SeenHead>,
    pub playing: Vec<Playing>,
}

fn read_dir_names(dir: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error(dir.to_path_buf(), e)),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    Ok(names)
}

/// Reads `<folder>/<uid>/`. Damaged or foreign files are ignored (another
/// launcher version, a sync client's leftovers).
pub fn read_folder(dir: &Path, uid: &str) -> Result<FolderView> {
    let mut view = FolderView::default();
    for name in read_dir_names(dir)? {
        let path = dir.join(&name);
        if let Some(machine) = name
            .strip_prefix("head-")
            .and_then(|n| n.strip_suffix(".json"))
        {
            let Ok(Some(head)) = read_json::<Head>(&path) else {
                continue;
            };
            if head.format != FORMAT
                || head.uid != uid
                || head.machine_id != machine
                || parse_state_name(&head.file).map(|(g, m)| (g, m.to_owned()))
                    != Some((head.generation, head.machine_id.clone()))
            {
                continue;
            }
            let arrived = std::fs::metadata(dir.join(&head.file))
                .is_ok_and(|m| m.is_file() && m.len() == head.size);
            view.heads.push(SeenHead { head, arrived });
        } else if name.starts_with("playing-") && name.ends_with(".json") {
            if let Ok(Some(playing)) = read_json::<Playing>(&path) {
                view.playing.push(playing);
            }
        }
    }
    view.heads.sort_by(|a, b| newer(&b.head, &a.head));
    Ok(view)
}

/// Order of heads: generation, then time, then machine id.
fn newer(a: &Head, b: &Head) -> std::cmp::Ordering {
    a.generation
        .cmp(&b.generation)
        .then(a.secs.cmp(&b.secs))
        .then_with(|| a.machine_id.cmp(&b.machine_id))
}

// ------------------------------------------------------------ decision

/// What to do with a shared realm (ADR 0028 decision table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    UpToDate,
    /// Played here since the last handoff: send at the next stop.
    ToSend,
    /// The other computer's newer state is here: offer to take it back.
    ToTake(Head),
    /// A newer state is announced, its file is not there yet.
    Arriving(Head),
    /// Both computers played since the last handoff.
    Conflict(Head),
}

/// The decision for `link`, given the folder's heads (newest first).
pub fn decide(link: &Link, heads: &[SeenHead]) -> Decision {
    let newest = heads.first();
    match newest {
        Some(seen) if seen.head.generation > link.generation => {
            if link.played {
                Decision::Conflict(seen.head.clone())
            } else if seen.arrived {
                Decision::ToTake(seen.head.clone())
            } else {
                // An older arrived state of another computer is not offered:
                // the newest one is on its way.
                Decision::Arriving(seen.head.clone())
            }
        }
        _ if link.played => Decision::ToSend,
        _ => Decision::UpToDate,
    }
}

/// Another computer's playing mark young enough to warn about.
pub fn other_playing(view: &FolderView, machine_id: &str, now: u64) -> Option<Playing> {
    view.playing
        .iter()
        .filter(|p| p.machine_id != machine_id && p.secs.saturating_add(PLAYING_FRESH_SECS) > now)
        .max_by_key(|p| p.secs)
        .cloned()
}

/// Another computer's older playing mark that its sends never cleared: it
/// played after its newest state (the launcher or the computer stopped
/// before the send), so the folder's state misses that game.
pub fn other_unsent(view: &FolderView, machine_id: &str, now: u64) -> Option<Playing> {
    view.playing
        .iter()
        .filter(|p| {
            p.machine_id != machine_id
                && p.secs.saturating_add(PLAYING_FRESH_SECS) <= now
                && !view
                    .heads
                    .iter()
                    .any(|s| s.head.machine_id == p.machine_id && s.head.secs >= p.secs)
        })
        .max_by_key(|p| p.secs)
        .cloned()
}

// ------------------------------------------------------------ status

/// A head as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub machine: String,
    pub secs: u64,
    pub generation: u64,
}

impl From<&Head> for Remote {
    fn from(head: &Head) -> Self {
        Self {
            machine: head.machine.clone(),
            secs: head.secs,
            generation: head.generation,
        }
    }
}

/// The handoff state of one realm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum State {
    /// The realm is not shared.
    NotShared,
    /// Shared, but no handoff folder is chosen on this computer.
    NoFolder,
    /// The folder cannot be read (sync client stopped, drive missing).
    Unavailable,
    UpToDate,
    ToSend,
    ToTake {
        from: Remote,
    },
    Arriving {
        from: Remote,
    },
    Conflict {
        from: Remote,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[serde(flatten)]
    pub state: State,
    /// Another computer's servers seem to run on this realm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_playing: Option<Remote>,
    /// Another computer played this realm after its newest state and never
    /// sent that game (its launcher has to send it first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_unsent: Option<Remote>,
    /// Time of the newest state this computer sent, when it is the newest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent: Option<u64>,
}

impl Status {
    fn only(state: State) -> Self {
        Self {
            state,
            other_playing: None,
            other_unsent: None,
            sent: None,
        }
    }
}

/// A shared realm of the folder, for the list of realms to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedRealm {
    pub uid: String,
    pub emulator: String,
    pub name: Option<String>,
    pub from: Remote,
    /// Its newest state file is there.
    pub arrived: bool,
    /// The local realm sharing it, if any.
    pub local: Option<String>,
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|e| io_error(path.to_path_buf(), e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| io_error(path.to_path_buf(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Copies `from` to `to` atomically (`to.tmp`, then renamed).
fn copy_atomic(from: &Path, to: &Path) -> Result<()> {
    let tmp = to.with_extension("tmp");
    let copied = (|| -> std::io::Result<()> {
        let mut source = std::fs::File::open(from)?;
        let mut target = std::fs::File::create(&tmp)?;
        std::io::copy(&mut source, &mut target)?;
        target.flush()?;
        target.sync_all()
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_error(to.to_path_buf(), e));
    }
    std::fs::rename(&tmp, to).map_err(|e| io_error(to.to_path_buf(), e))
}

/// `x.y.z` as numbers, for comparing launcher versions.
fn version(text: &str) -> Vec<u64> {
    text.split(['.', '-'])
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

impl Launcher {
    fn link_path(&self, id: &str) -> PathBuf {
        self.layout.instance(id).root.join(LINK_FILE)
    }

    /// The realm's link to the folder, `None` when it is not shared.
    pub fn handoff_link(&self, id: &str) -> Result<Option<Link>> {
        read_json(&self.link_path(id))
    }

    pub(crate) fn save_link(&self, id: &str, link: &Link) -> Result<()> {
        write_json_atomic(&self.link_path(id), link)
    }

    fn realm_dir(place: &Place, uid: &str) -> PathBuf {
        place.folder.join(uid)
    }

    /// The handoff state of realm `id` (`place`: this computer's folder).
    pub fn handoff_status(&self, id: &str, place: Option<&Place>) -> Result<Status> {
        install::open_existing(&self.layout, id)?;
        let Some(link) = self.handoff_link(id)? else {
            return Ok(Status::only(State::NotShared));
        };
        let Some(place) = place else {
            return Ok(Status::only(State::NoFolder));
        };
        if !place.folder.is_dir() {
            return Ok(Status::only(State::Unavailable));
        }
        let Ok(view) = read_folder(&Self::realm_dir(place, &link.uid), &link.uid) else {
            return Ok(Status::only(State::Unavailable));
        };
        let state = match decide(&link, &view.heads) {
            Decision::UpToDate => State::UpToDate,
            Decision::ToSend => State::ToSend,
            Decision::ToTake(head) => State::ToTake {
                from: Remote::from(&head),
            },
            Decision::Arriving(head) => State::Arriving {
                from: Remote::from(&head),
            },
            Decision::Conflict(head) => State::Conflict {
                from: Remote::from(&head),
            },
        };
        let sent = view
            .heads
            .first()
            .filter(|s| {
                s.head.machine_id == place.machine_id && s.head.generation == link.generation
            })
            .map(|s| s.head.secs);
        let now = unix_secs();
        Ok(Status {
            state,
            other_playing: other_playing(&view, &place.machine_id, now).map(|p| Remote {
                machine: p.machine,
                secs: p.secs,
                generation: 0,
            }),
            other_unsent: other_unsent(&view, &place.machine_id, now).map(|p| Remote {
                machine: p.machine,
                secs: p.secs,
                generation: 0,
            }),
            sent,
        })
    }

    /// Shares the stopped realm `id` through the folder: a share id, then
    /// generation 1 sent at once.
    pub async fn handoff_share(&self, id: &str, place: &Place) -> Result<Head> {
        let summary = self.instance(id)?;
        if !summary.installed || summary.unfinished.is_some() {
            return Err(install::PlanError::Unfinished(id.to_owned()).into());
        }
        if self.handoff_link(id)?.is_none() {
            self.save_link(
                id,
                &Link {
                    format: FORMAT,
                    uid: random_hex(16)?,
                    generation: 0,
                    played: true,
                    pulling: None,
                },
            )?;
        }
        self.handoff_send(id, place, true)
            .await?
            .ok_or_else(|| LauncherError::Invalid("the realm was not sent".into()))
    }

    /// Stops sharing `id` here (the folder keeps its files).
    pub fn handoff_unshare(&self, id: &str) -> Result<()> {
        install::open_existing(&self.layout, id)?;
        match std::fs::remove_file(self.link_path(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(io_error(self.link_path(id), e))
            }
            _ => Ok(()),
        }
    }

    /// The servers of `id` started on this computer: it is played here
    /// and the other computers are told (best effort for the mark).
    pub fn handoff_started(&self, id: &str, place: Option<&Place>) -> Result<()> {
        let Some(mut link) = self.handoff_link(id)? else {
            return Ok(());
        };
        if !link.played {
            link.played = true;
            self.save_link(id, &link)?;
        }
        if let Some(place) = place {
            let dir = Self::realm_dir(place, &link.uid);
            let mark = Playing {
                machine_id: place.machine_id.clone(),
                machine: place.machine.clone(),
                secs: unix_secs(),
            };
            let _ = write_json_atomic(
                &dir.join(format!("playing-{}.json", place.machine_id)),
                &mark,
            );
        }
        Ok(())
    }

    /// Sends the stopped realm's player data to the folder when it was
    /// played here (`force`: also when not, and over a conflict: "keep this
    /// computer's game"). `None` when there was nothing to send. Refused
    /// while the other computer's newer state is waiting (it would be
    /// hidden) unless forced.
    pub async fn handoff_send(&self, id: &str, place: &Place, force: bool) -> Result<Option<Head>> {
        let Some(mut link) = self.handoff_link(id)? else {
            return Err(LauncherError::HandoffNotShared(id.to_owned()));
        };
        let summary = self.instance(id)?;
        if !summary.installed || summary.unfinished.is_some() || link.pulling.is_some() {
            return Err(install::PlanError::Unfinished(id.to_owned()).into());
        }
        if !place.folder.is_dir() {
            return Err(LauncherError::HandoffUnavailable(place.folder.clone()));
        }
        let dir = Self::realm_dir(place, &link.uid);
        let view = read_folder(&dir, &link.uid)?;
        match decide(&link, &view.heads) {
            Decision::UpToDate if !force => return Ok(None),
            Decision::ToTake(head) | Decision::Conflict(head) | Decision::Arriving(head)
                if !force && head.generation > link.generation =>
            {
                return Err(LauncherError::HandoffBehind(head.machine));
            }
            _ => {}
        }
        let newest = view.heads.first().map_or(0, |s| s.head.generation);
        let generation = link.generation.max(newest) + 1;
        std::fs::create_dir_all(&dir).map_err(|e| io_error(dir.clone(), e))?;
        let file = state_name(generation, &place.machine_id);
        let path = dir.join(&file);
        self.export(id, &path).await?;
        let (size, sha256) = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || -> Result<(u64, String)> {
                let size = std::fs::metadata(&path)
                    .map_err(|e| io_error(path.clone(), e))?
                    .len();
                Ok((size, sha256_file(&path)?))
            })
            .await
            .map_err(|e| LauncherError::Invalid(e.to_string()))??
        };
        let record = summary.record;
        let head = Head {
            format: FORMAT,
            uid: link.uid.clone(),
            generation,
            parent: link.generation,
            machine_id: place.machine_id.clone(),
            machine: place.machine.clone(),
            secs: unix_secs(),
            emulator: record.emulator.clone(),
            file,
            size,
            sha256,
            build: record.build.clone(),
            launcher: env!("CARGO_PKG_VERSION").to_owned(),
            name: record.name.clone(),
        };
        write_json_atomic(&dir.join(format!("head-{}.json", place.machine_id)), &head)?;
        let _ = std::fs::remove_file(dir.join(format!("playing-{}.json", place.machine_id)));
        link.generation = generation;
        link.played = false;
        self.save_link(id, &link)?;
        prune_states(&dir, &place.machine_id);
        Ok(Some(head))
    }

    /// Takes back the other computer's newer state of `id` (ADR 0028): its
    /// choices (modules, options, realm settings) when they differ, then
    /// its player data (a restore: the current data is dumped first).
    /// Resumable with [`Launcher::resume`].
    pub async fn handoff_take(
        &self,
        id: &str,
        place: &Place,
        observer: &dyn Observer,
        cancel: &CancelToken,
    ) -> Result<InstanceRecord> {
        let Some(mut link) = self.handoff_link(id)? else {
            return Err(LauncherError::HandoffNotShared(id.to_owned()));
        };
        let summary = self.instance(id)?;
        if !summary.installed || summary.unfinished.is_some() {
            return Err(install::PlanError::Unfinished(id.to_owned()).into());
        }
        if !place.folder.is_dir() {
            return Err(LauncherError::HandoffUnavailable(place.folder.clone()));
        }
        let dir = Self::realm_dir(place, &link.uid);
        let view = read_folder(&dir, &link.uid)?;
        let head = match decide(&link, &view.heads) {
            Decision::ToTake(head) => head,
            Decision::Conflict(head) => {
                let arrived = view.heads.first().is_some_and(|s| s.arrived);
                if !arrived {
                    return Err(LauncherError::HandoffNotArrived(head.machine));
                }
                head
            }
            Decision::Arriving(head) => return Err(LauncherError::HandoffNotArrived(head.machine)),
            Decision::UpToDate | Decision::ToSend => {
                return Err(LauncherError::HandoffNothingToTake)
            }
        };
        let state = dir.join(&head.file);
        // A copy first: the sync client may replace the folder's file.
        let local = self.layout.instance(id).root.join("handoff-state.tmp");
        let checked = {
            let (state, local, sha) = (state.clone(), local.clone(), head.sha256.clone());
            tokio::task::spawn_blocking(move || -> Result<bool> {
                copy_atomic(&state, &local)?;
                Ok(sha256_file(&local)? == sha)
            })
            .await
            .map_err(|e| LauncherError::Invalid(e.to_string()))?
        };
        let result = match checked {
            Ok(true) => {
                self.take_state(id, &mut link, &head, &local, observer, cancel)
                    .await
            }
            Ok(false) => Err(LauncherError::HandoffNotArrived(head.machine.clone())),
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&local);
        if result.is_ok() {
            // This computer's game was replaced: its playing mark would tell
            // the other one it played without sending (`other_unsent`).
            let _ = std::fs::remove_file(dir.join(format!("playing-{}.json", place.machine_id)));
        }
        result
    }

    async fn take_state(
        &self,
        id: &str,
        link: &mut Link,
        head: &Head,
        state: &Path,
        observer: &dyn Observer,
        cancel: &CancelToken,
    ) -> Result<InstanceRecord> {
        let (realm, _, _) = realms::read_export_head(state)?;
        if version(&realm.launcher) > version(env!("CARGO_PKG_VERSION")) {
            return Err(LauncherError::HandoffLauncherOlder(realm.launcher));
        }
        self.check_importable(&realm)?;
        let (record, _) = install::open_existing(&self.layout, id)?;
        if realm.emulator != record.emulator || realm.flavor != record.flavor {
            return Err(LauncherError::ExportUnsupported(format!(
                "{} {}",
                realm.emulator, realm.flavor
            )));
        }
        // Player data never goes to an older schema. Builds have no order:
        // the game was made on the build this realm is offered as an update
        // (the other computer updated first). A newer launcher there was
        // refused above; an older build there is upgraded by the restore's
        // database update.
        if realm.build.is_some() && realm.build != record.build {
            if let ServerUpdate::Available { build, .. } = self.server_update(id).await? {
                if realm.build.as_deref() == Some(build.as_str()) {
                    return Err(LauncherError::HandoffUpdateFirst);
                }
            }
        }
        let _lock = install::lock_instance(&self.layout, id)?;
        link.pulling = Some(head.generation);
        self.save_link(id, link)?;
        let result = self
            .pull(id, record, &realm, head, state, observer, cancel)
            .await;
        match &result {
            // Interrupted: `resume` finishes the restore, then the take.
            Err(LauncherError::Cancelled(_)) if self.take_pending(id)? => {}
            // Failed (rolled back): this computer keeps its own game and
            // can take the other one again later.
            Err(_) => self.abandon_take(id)?,
            Ok(_) => {}
        }
        result
    }

    /// The take back proper, once `pulling` is saved.
    #[allow(clippy::too_many_arguments)]
    async fn pull(
        &self,
        id: &str,
        record: InstanceRecord,
        realm: &realms::ExportedRealm,
        head: &Head,
        state: &Path,
        observer: &dyn Observer,
        cancel: &CancelToken,
    ) -> Result<InstanceRecord> {
        let name = format!("handoff-{}-{}.sql", head.generation, unix_secs());
        let backups_dir = self.layout.backups(id);
        std::fs::create_dir_all(&backups_dir).map_err(|e| io_error(backups_dir.clone(), e))?;
        let dump = backups_dir.join(&name);
        tokio::task::block_in_place(|| realms::extract_dump(state, &dump))?;
        let mut record = record;
        if record.modules != realm.modules || record.options != realm.options {
            let (new, old) = rk_runtime::modules::set_choices(
                &self.catalog,
                &self.layout,
                id,
                &realm.modules,
                &realm.options,
            )?;
            record = self.apply(new, &old, observer, cancel).await?;
        }
        let record = self
            .restore_locked(record, dump, &name, observer, cancel)
            .await?;
        self.finish_take(id)?;
        let _ = backups::prune(&backups_dir);
        Ok(record)
    }

    /// Whether the change waiting to be resumed is the restore of a take
    /// back (its dump is a `handoff-` backup).
    pub(crate) fn take_pending(&self, id: &str) -> Result<bool> {
        Ok(matches!(
            self.pending(id)?,
            Some(Pending::Restore { dump, .. })
                if dump.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("handoff-"))
        ))
    }

    /// A take back that will not finish: the realm keeps its own game, the
    /// other computer's stays to take (the restore, if it ran, was rolled
    /// back; a restore left unfinished keeps its pending change).
    pub(crate) fn abandon_take(&self, id: &str) -> Result<()> {
        if self.take_pending(id)?
            && install::ensure_finished(&self.layout, &[install::instance_operation_id(id)]).is_ok()
        {
            self.clear_pending(id)?;
        }
        let Some(mut link) = self.handoff_link(id)? else {
            return Ok(());
        };
        if link.pulling.take().is_some() {
            self.save_link(id, &link)?;
        }
        Ok(())
    }

    /// A take back that ended (also through `resume`): the realm is at
    /// that generation and not played since.
    pub(crate) fn finish_take(&self, id: &str) -> Result<()> {
        let Some(mut link) = self.handoff_link(id)? else {
            return Ok(());
        };
        if let Some(generation) = link.pulling.take() {
            link.generation = generation;
            link.played = false;
            self.save_link(id, &link)?;
        }
        Ok(())
    }

    /// Realms shared in the folder, with the local realm sharing each.
    pub fn handoff_shared(&self, place: &Place) -> Result<Vec<SharedRealm>> {
        if !place.folder.is_dir() {
            return Err(LauncherError::HandoffUnavailable(place.folder.clone()));
        }
        let mut local = BTreeMap::new();
        for summary in self.instances()? {
            if let Ok(Some(link)) = self.handoff_link(&summary.record.id) {
                local.insert(link.uid, summary.record.id.clone());
            }
        }
        let mut out = Vec::new();
        for uid in read_dir_names(&place.folder)? {
            if !valid_uid(&uid) {
                continue;
            }
            let Ok(view) = read_folder(&place.folder.join(&uid), &uid) else {
                continue;
            };
            let Some(newest) = view.heads.first() else {
                continue;
            };
            out.push(SharedRealm {
                emulator: newest.head.emulator.clone(),
                name: newest.head.name.clone(),
                from: Remote::from(&newest.head),
                arrived: newest.arrived,
                local: local.get(&uid).cloned(),
                uid,
            });
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.from.secs));
        Ok(out)
    }

    /// Installs the folder's shared realm `uid` as the new realm `id` (an
    /// import of its newest state) linked to it. Resumable like an import.
    #[allow(clippy::too_many_arguments)]
    pub async fn handoff_join(
        &self,
        place: &Place,
        uid: &str,
        id: &str,
        name: Option<&str>,
        index: Option<&str>,
        observer: &dyn Observer,
        cancel: &CancelToken,
    ) -> Result<InstanceRecord> {
        if !valid_uid(uid) {
            return Err(LauncherError::HandoffUnknown(uid.to_owned()));
        }
        if let Some(local) = self
            .handoff_shared(place)?
            .into_iter()
            .find(|s| s.uid == uid)
        {
            if let Some(local) = local.local {
                return Err(LauncherError::InstanceExists(local));
            }
        }
        let dir = place.folder.join(uid);
        let view = read_folder(&dir, uid)?;
        let Some(newest) = view.heads.first() else {
            return Err(LauncherError::HandoffUnknown(uid.to_owned()));
        };
        let head = newest.head.clone();
        if !newest.arrived {
            return Err(LauncherError::HandoffNotArrived(head.machine));
        }
        self.check_new_id(id)?;
        let staging = self
            .layout
            .root
            .join("state")
            .join(format!("handoff-join-{id}.tmp"));
        let checked = {
            let (state, staging, sha) =
                (dir.join(&head.file), staging.clone(), head.sha256.clone());
            tokio::task::spawn_blocking(move || -> Result<bool> {
                if let Some(parent) = staging.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| io_error(parent.to_path_buf(), e))?;
                }
                copy_atomic(&state, &staging)?;
                Ok(sha256_file(&staging)? == sha)
            })
            .await
            .map_err(|e| LauncherError::Invalid(e.to_string()))?
        };
        let result = match checked {
            Ok(true) => {
                let link = Link {
                    format: FORMAT,
                    uid: uid.to_owned(),
                    generation: head.generation,
                    played: false,
                    pulling: None,
                };
                self.import_linked(&staging, id, name, index, Some(link), observer, cancel)
                    .await
            }
            Ok(false) => Err(LauncherError::HandoffNotArrived(head.machine.clone())),
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&staging);
        result
    }
}

/// Deletes this computer's state files older than the newest
/// [`KEEP_GENERATIONS`] generations of the folder (best effort; the other
/// computers prune their own).
fn prune_states(dir: &Path, machine_id: &str) {
    let Ok(names) = read_dir_names(dir) else {
        return;
    };
    let mut generations: Vec<u64> = names
        .iter()
        .filter_map(|n| parse_state_name(n).map(|(g, _)| g))
        .collect();
    generations.sort_unstable_by(|a, b| b.cmp(a));
    generations.dedup();
    let Some(&keep_from) = generations.get(KEEP_GENERATIONS - 1) else {
        return;
    };
    for name in &names {
        if let Some((generation, machine)) = parse_state_name(name) {
            if machine == machine_id && generation < keep_from {
                let _ = std::fs::remove_file(dir.join(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "0123456789abcdef";
    const OTHER: &str = "fedcba9876543210";

    fn link(generation: u64, played: bool) -> Link {
        Link {
            format: FORMAT,
            uid: "a".repeat(32),
            generation,
            played,
            pulling: None,
        }
    }

    fn head(generation: u64, machine_id: &str, secs: u64) -> Head {
        Head {
            format: FORMAT,
            uid: "a".repeat(32),
            generation,
            parent: generation.saturating_sub(1),
            machine_id: machine_id.into(),
            machine: format!("PC-{}", &machine_id[..2]),
            secs,
            emulator: "azerothcore".into(),
            file: state_name(generation, machine_id),
            size: 3,
            sha256: String::new(),
            build: None,
            launcher: "0.3.4".into(),
            name: Some("Garona".into()),
        }
    }

    fn seen(head: Head, arrived: bool) -> SeenHead {
        SeenHead { head, arrived }
    }

    #[test]
    fn the_decision_follows_the_table() {
        // Nothing in the folder.
        assert_eq!(decide(&link(0, false), &[]), Decision::UpToDate);
        assert_eq!(decide(&link(0, true), &[]), Decision::ToSend);
        // This computer's own head is the newest.
        let mine = [seen(head(3, ME, 30), true)];
        assert_eq!(decide(&link(3, false), &mine), Decision::UpToDate);
        assert_eq!(decide(&link(3, true), &mine), Decision::ToSend);
        // The other computer played after it.
        let other = head(4, OTHER, 40);
        let heads = [seen(other.clone(), true), seen(head(3, ME, 30), true)];
        assert_eq!(
            decide(&link(3, false), &heads),
            Decision::ToTake(other.clone())
        );
        assert_eq!(
            decide(&link(3, true), &heads),
            Decision::Conflict(other.clone())
        );
        // Announced but not arrived: never an older state instead.
        let heads = [seen(other.clone(), false), seen(head(3, ME, 30), true)];
        assert_eq!(
            decide(&link(3, false), &heads),
            Decision::Arriving(other.clone())
        );
        assert_eq!(decide(&link(3, true), &heads), Decision::Conflict(other));
        // A folder behind this computer (its files not synchronised yet).
        let behind = [seen(head(2, OTHER, 20), true)];
        assert_eq!(decide(&link(3, false), &behind), Decision::UpToDate);
        assert_eq!(decide(&link(3, true), &behind), Decision::ToSend);
    }

    #[test]
    fn heads_are_read_newest_first_and_checked() {
        let root = tempfile::tempdir().unwrap();
        let uid = "a".repeat(32);
        let dir = root.path().join(&uid);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |h: &Head| {
            std::fs::write(
                dir.join(format!("head-{}.json", h.machine_id)),
                serde_json::to_vec(h).unwrap(),
            )
            .unwrap();
        };
        let mine = head(1, ME, 10);
        write(&mine);
        std::fs::write(dir.join(&mine.file), b"abc").unwrap();
        let other = head(2, OTHER, 20);
        write(&other);
        std::fs::write(dir.join(&other.file), b"ab").unwrap(); // not all there
                                                               // A head of another realm and a damaged one are ignored.
        let mut foreign = head(9, "1111111111111111", 90);
        foreign.uid = "b".repeat(32);
        write(&foreign);
        std::fs::write(dir.join("head-2222222222222222.json"), b"{").unwrap();
        let playing = Playing {
            machine_id: OTHER.into(),
            machine: "PC-fe".into(),
            secs: 1_000,
        };
        std::fs::write(
            dir.join(format!("playing-{OTHER}.json")),
            serde_json::to_vec(&playing).unwrap(),
        )
        .unwrap();
        let view = read_folder(&dir, &uid).unwrap();
        assert_eq!(view.heads, vec![seen(other, false), seen(mine, true)]);
        assert_eq!(other_playing(&view, ME, 2_000), Some(playing.clone()));
        assert_eq!(other_playing(&view, OTHER, 2_000), None, "not my own mark");
        assert_eq!(other_playing(&view, ME, 1_000 + PLAYING_FRESH_SECS), None);
        // Played after its newest state (secs 20) and never sent.
        assert_eq!(other_unsent(&view, ME, 2_000), None, "still fresh");
        assert_eq!(
            other_unsent(&view, ME, 1_000 + PLAYING_FRESH_SECS),
            Some(playing.clone())
        );
    }

    #[test]
    fn a_mark_older_than_its_computers_state_is_not_unsent() {
        let mut view = FolderView::default();
        view.heads.push(seen(head(3, OTHER, 5_000), true));
        view.playing.push(Playing {
            machine_id: OTHER.into(),
            machine: "PC-fe".into(),
            secs: 4_000,
        });
        assert_eq!(other_unsent(&view, ME, 4_000 + PLAYING_FRESH_SECS), None);
    }

    #[test]
    fn only_my_older_states_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        for (generation, machine) in [(1, ME), (2, OTHER), (3, ME), (4, OTHER), (5, ME)] {
            std::fs::write(dir.path().join(state_name(generation, machine)), b"x").unwrap();
        }
        prune_states(dir.path(), ME);
        let left = read_dir_names(dir.path()).unwrap();
        assert_eq!(
            left,
            vec![
                state_name(2, OTHER),
                state_name(4, OTHER),
                state_name(5, ME)
            ]
        );
    }

    #[test]
    fn state_names_round_trip() {
        let name = state_name(12, ME);
        assert_eq!(parse_state_name(&name), Some((12, ME)));
        assert_eq!(parse_state_name("state-x-0123456789abcdef.rkrealm"), None);
        assert_eq!(parse_state_name("state-1-short.rkrealm"), None);
        assert!(version("0.3.10") > version("0.3.9"));
        assert!(version("0.4.0") > version("0.3.99"));
    }

    #[test]
    fn the_folder_must_stay_out_of_the_data_root() {
        let base = tempfile::tempdir().unwrap();
        let data = base.path().join("data");
        let sync = base.path().join("sync");
        std::fs::create_dir_all(data.join("inner")).unwrap();
        std::fs::create_dir_all(&sync).unwrap();
        check_folder(&sync, &data).unwrap();
        assert!(check_folder(&data.join("inner"), &data).is_err());
        assert!(check_folder(base.path(), &data).is_err());
        assert!(check_folder(&base.path().join("missing"), &data).is_err());
    }

    #[test]
    fn the_machine_id_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = settings(dir.path()).unwrap();
        assert!(valid_machine(&first.machine_id));
        assert_eq!(settings(dir.path()).unwrap(), first);
        assert!(place(dir.path()).unwrap().is_none());
    }
}
