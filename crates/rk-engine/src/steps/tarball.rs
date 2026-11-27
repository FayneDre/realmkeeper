//! `.tar.xz` archives (the Linux MySQL download) and the files of Debian
//! packages (libraries some Linux systems lack), ADR 0031.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lzma_rust2::XzReader;

use super::extract::{io, is_relative_name, set_mode, ExtractError, Select, Strip};
use crate::CancelToken;

/// Whether `path` starts with the xz magic bytes.
pub(crate) fn is_xz(path: &Path) -> Result<bool, ExtractError> {
    let mut magic = [0u8; 6];
    let mut file = File::open(path).map_err(io(path))?;
    let read = file.read(&mut magic).map_err(io(path))?;
    Ok(read == magic.len() && magic == [0xFD, b'7', b'z', b'X', b'Z', 0])
}

/// Counts the compressed bytes read: the progress of a stream whose
/// unpacked size is unknown until its end.
struct Counting<'a, R> {
    inner: R,
    done: &'a AtomicU64,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Path components of a tar entry name, without `./` and empty parts;
/// `None` for an unsafe name (absolute, `..`, drive).
fn components(name: &str) -> Option<Vec<String>> {
    if !is_relative_name(name) {
        return None;
    }
    Some(
        name.split(['/', '\\'])
            .filter(|p| !p.is_empty() && *p != ".")
            .map(str::to_owned)
            .collect(),
    )
}

fn name_of(bytes: &[u8]) -> Result<String, ExtractError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ExtractError::UnsafeEntry(String::from_utf8_lossy(bytes).into_owned()))
}

/// What an entry becomes once its path is selected and stripped.
enum Kind {
    Dir,
    File,
    /// Symbolic or hard link to another file of the archive (relative path).
    Link(Vec<String>),
}

/// Extracts a `.tar.xz` archive into the empty directory `dest`, in one
/// pass (the stream cannot be read twice cheaply): every entry is checked
/// before it is written and a failure leaves only the staging directory,
/// which is never put in place. Links become hard links to the file they
/// name, which must be a file of the same archive: nothing written can point
/// outside `dest`. Returns `false` when cancelled.
pub(crate) fn untar_xz(
    archive: &Path,
    dest: &Path,
    Select { subdir, strip }: Select<'_>,
    (done, total): (&AtomicU64, &AtomicU64),
    cancel: &CancelToken,
) -> Result<bool, ExtractError> {
    let file = File::open(archive).map_err(io(archive))?;
    total.store(
        file.metadata().map_err(io(archive))?.len(),
        Ordering::Relaxed,
    );
    let reader = Counting {
        inner: BufReader::with_capacity(1 << 20, file),
        done,
    };
    let mut tar = tar::Archive::new(XzReader::new(reader, true));
    let invalid = |e: std::io::Error| ExtractError::Io {
        path: archive.to_path_buf(),
        source: e,
    };
    let prefix: Vec<String> = subdir
        .map(|s| {
            s.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();

    let mut root: Option<String> = None;
    // Relative (to `dest`) path -> what it is, for duplicates and links.
    let mut written: BTreeMap<Vec<String>, bool> = BTreeMap::new(); // true = file
    let mut seen = BTreeSet::new();
    let mut links: Vec<(Vec<String>, Vec<String>, String)> = Vec::new();
    let mut buf = vec![0u8; 256 * 1024];

    // Selects and strips an archive path; `None` when it is left out.
    let mut place =
        |parts: Vec<String>, is_dir: bool| -> Result<Option<Vec<String>>, ExtractError> {
            let parts = if prefix.is_empty() {
                parts
            } else if parts.len() > prefix.len() && parts[..prefix.len()] == prefix[..] {
                parts[prefix.len()..].to_vec()
            } else {
                return Ok(None);
            };
            let skip = match strip {
                Strip::None => 0,
                Strip::Components(n) => n,
                Strip::SingleRootDir => {
                    let Some(first) = parts.first() else {
                        return Ok(None);
                    };
                    match &root {
                        Some(r) if r != first => return Err(ExtractError::NoSingleRoot),
                        Some(_) => {}
                        None => root = Some(first.clone()),
                    }
                    if !is_dir && parts.len() == 1 {
                        return Err(ExtractError::NoSingleRoot);
                    }
                    1
                }
            };
            Ok(parts
                .get(skip..)
                .filter(|p| !p.is_empty())
                .map(<[String]>::to_vec))
        };

    for entry in tar.entries().map_err(invalid)? {
        if cancel.is_cancelled() {
            return Ok(false);
        }
        let mut entry = entry.map_err(invalid)?;
        let name = name_of(&entry.path_bytes())?;
        let kind = entry.header().entry_type();
        use tar::EntryType as T;
        let parts = components(&name).ok_or_else(|| ExtractError::UnsafeEntry(name.clone()))?;
        let what = match kind {
            T::Directory => Kind::Dir,
            T::Regular | T::Continuous => Kind::File,
            T::XGlobalHeader | T::XHeader | T::GNULongName | T::GNULongLink => continue,
            T::Symlink => {
                let target = entry
                    .link_name_bytes()
                    .map(|b| name_of(&b))
                    .transpose()?
                    .ok_or_else(|| ExtractError::UnsafeEntry(name.clone()))?;
                // Relative to the link's directory, resolved inside the archive.
                if target.starts_with('/') || target.contains(['\\', ':']) {
                    return Err(ExtractError::UnsafeEntry(name));
                }
                let mut resolved = parts[..parts.len().saturating_sub(1)].to_vec();
                for part in target.split('/').filter(|p| !p.is_empty() && *p != ".") {
                    if part == ".." {
                        if resolved.pop().is_none() {
                            return Err(ExtractError::UnsafeEntry(name));
                        }
                    } else {
                        resolved.push(part.to_owned());
                    }
                }
                Kind::Link(resolved)
            }
            T::Link => {
                let target = entry
                    .link_name_bytes()
                    .map(|b| name_of(&b))
                    .transpose()?
                    .ok_or_else(|| ExtractError::UnsafeEntry(name.clone()))?;
                Kind::Link(
                    components(&target).ok_or_else(|| ExtractError::UnsafeEntry(name.clone()))?,
                )
            }
            // Devices, FIFOs, ...: never part of a program download.
            _ => return Err(ExtractError::UnsafeEntry(name)),
        };
        let Some(relative) = place(parts, matches!(what, Kind::Dir))? else {
            continue;
        };
        let out: PathBuf = relative.iter().fold(dest.to_path_buf(), |p, c| p.join(c));
        if matches!(what, Kind::Dir) {
            std::fs::create_dir_all(&out).map_err(io(&out))?;
            continue;
        }
        let key = if cfg!(windows) {
            out.to_string_lossy().to_lowercase()
        } else {
            out.to_string_lossy().into_owned()
        };
        if !seen.insert(key) {
            return Err(ExtractError::Duplicate(out.display().to_string()));
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        match what {
            Kind::Link(target) => {
                // An archive path (a symbolic link's was resolved against
                // its directory): selected and stripped like the entries.
                let target = place(target, false)?;
                let target = target.ok_or_else(|| ExtractError::UnsafeEntry(name.clone()))?;
                links.push((relative.clone(), target, name));
                written.insert(relative, false);
            }
            Kind::File => {
                let mode = entry.header().mode().ok();
                let mut file = File::create(&out).map_err(io(&out))?;
                loop {
                    if cancel.is_cancelled() {
                        return Ok(false);
                    }
                    let n = entry.read(&mut buf).map_err(io(&out))?;
                    if n == 0 {
                        break;
                    }
                    file.write_all(&buf[..n]).map_err(io(&out))?;
                }
                drop(file);
                if let Some(mode) = mode {
                    set_mode(&out, mode).map_err(io(&out))?;
                }
                written.insert(relative, true);
            }
            Kind::Dir => unreachable!(),
        }
    }

    // Links last: their files are all written now. A chain of links ends on
    // a file of the archive, or the archive is refused.
    let targets: BTreeMap<&Vec<String>, &Vec<String>> =
        links.iter().map(|(l, t, _)| (l, t)).collect();
    for (link, target, name) in &links {
        let mut file = target;
        for _ in 0..=links.len() {
            if written.get(file) == Some(&true) {
                break;
            }
            match targets.get(file) {
                Some(next) => file = next,
                None => return Err(ExtractError::UnsafeEntry(name.clone())),
            }
        }
        if written.get(file) != Some(&true) {
            return Err(ExtractError::UnsafeEntry(name.clone())); // a cycle
        }
        let from = file.iter().fold(dest.to_path_buf(), |p, c| p.join(c));
        let to = link.iter().fold(dest.to_path_buf(), |p, c| p.join(c));
        std::fs::hard_link(&from, &to).map_err(io(&to))?;
    }
    Ok(!cancel.is_cancelled())
}

/// The `data.tar` member of a Debian package (an `ar` archive), decompressed.
fn deb_data(package: &Path) -> Result<Box<dyn Read>, ExtractError> {
    let bad = |message: &str| ExtractError::Package {
        path: package.to_path_buf(),
        message: message.to_owned(),
    };
    let mut file = BufReader::new(File::open(package).map_err(io(package))?);
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic).map_err(io(package))?;
    if &magic != b"!<arch>\n" {
        return Err(bad("not a Debian package"));
    }
    loop {
        let mut header = [0u8; 60];
        if file.read_exact(&mut header).is_err() {
            return Err(bad("no data.tar member"));
        }
        if &header[58..60] != b"`\n" {
            return Err(bad("damaged member header"));
        }
        let name = String::from_utf8_lossy(&header[..16])
            .trim_end()
            .trim_end_matches('/')
            .to_owned();
        let size: u64 = String::from_utf8_lossy(&header[48..58])
            .trim()
            .parse()
            .map_err(|_| bad("damaged member size"))?;
        if let Some(compression) = name.strip_prefix("data.tar") {
            let member = file.take(size);
            return match compression {
                ".xz" => Ok(Box::new(XzReader::new(member, true))),
                "" => Ok(Box::new(member)),
                other => Err(bad(&format!("unsupported data.tar{other}"))),
            };
        }
        // Members are padded to an even size.
        file.seek(SeekFrom::Current((size + size % 2) as i64))
            .map_err(io(package))?;
    }
}

/// Copies `files` (path in the package, without `./` -> destination
/// relative to `dest`) out of a Debian package into `dest`, each through a
/// temporary name, so a rerun after an interruption simply writes them again.
pub(crate) fn unpack_package_files(
    package: &Path,
    dest: &Path,
    files: &BTreeMap<String, String>,
) -> Result<(), ExtractError> {
    for to in files.values() {
        if !is_relative_name(to) || components(to).is_none_or(|c| c.is_empty()) {
            return Err(ExtractError::UnsafeEntry(to.clone()));
        }
    }
    let mut tar = tar::Archive::new(deb_data(package)?);
    let invalid = |e: std::io::Error| ExtractError::Io {
        path: package.to_path_buf(),
        source: e,
    };
    let mut left: BTreeSet<&String> = files.keys().collect();
    for entry in tar.entries().map_err(invalid)? {
        let mut entry = entry.map_err(invalid)?;
        let name = name_of(&entry.path_bytes())?;
        let Some(parts) = components(&name) else {
            continue; // never one of ours: their names are plain
        };
        let name = parts.join("/");
        let Some((from, to)) = files.get_key_value(&name) else {
            continue;
        };
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err(ExtractError::Package {
                path: package.to_path_buf(),
                message: format!("'{name}' is not a regular file"),
            });
        }
        let out = dest.join(to);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let mut part = out.clone().into_os_string();
        part.push(".rk-part");
        let part = PathBuf::from(part);
        let mut file = File::create(&part).map_err(io(&part))?;
        std::io::copy(&mut entry, &mut file).map_err(io(&part))?;
        file.sync_all().map_err(io(&part))?;
        drop(file);
        set_mode(&part, 0o755).map_err(io(&part))?;
        std::fs::rename(&part, &out).map_err(io(&out))?;
        left.remove(from);
    }
    match left.first() {
        None => Ok(()),
        Some(file) => Err(ExtractError::Package {
            path: package.to_path_buf(),
            message: format!("'{file}' is missing"),
        }),
    }
}
