//! Linux downloads (ADR 0031): `.tar.xz` archives with links and modes, and
//! files taken out of Debian packages.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use lzma_rust2::{XzOptions, XzWriter};
use rk_engine::steps::{Extract, ExtractError, PackageFiles, Strip};
use rk_engine::{CancelToken, Engine, EngineError, Operation, Outcome};

enum Entry<'a> {
    Dir(&'a str),
    File(&'a str, &'a [u8], u32),
    Symlink(&'a str, &'a str),
}

fn tar_bytes(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        match entry {
            Entry::Dir(name) => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                tar.append_data(&mut header, name, std::io::empty())
                    .unwrap();
            }
            Entry::File(name, data, mode) => {
                header.set_mode(*mode);
                header.set_size(data.len() as u64);
                tar.append_data(&mut header, name, *data).unwrap();
            }
            Entry::Symlink(name, target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                tar.append_link(&mut header, name, target).unwrap();
            }
        }
    }
    tar.into_inner().unwrap()
}

fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut writer = XzWriter::new(Vec::new(), XzOptions::with_preset(1)).unwrap();
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

/// A Debian package: `ar` archive of `debian-binary`, `control.tar.xz`
/// and `data.tar.xz`.
fn deb(data: &[Entry<'_>]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    let members = [
        ("debian-binary", b"2.0\n".to_vec()),
        (
            "control.tar.xz",
            xz(&tar_bytes(&[Entry::File(
                "./control",
                b"Package: x\n",
                0o644,
            )])),
        ),
        ("data.tar.xz", xz(&tar_bytes(data))),
    ];
    for (name, body) in members {
        let header = format!(
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            0,
            0,
            0,
            "100644",
            body.len()
        );
        assert_eq!(header.len(), 60);
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&body);
        if body.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

async fn run(op: Operation<()>, state: &Path) -> Result<Outcome, EngineError> {
    Engine::new(state)
        .run(&op, &(), &(), &CancelToken::new())
        .await
}

fn extract_error(err: &EngineError) -> &ExtractError {
    let EngineError::StepFailed { source, .. } = err else {
        panic!("{err:?}")
    };
    source
        .downcast_ref::<ExtractError>()
        .unwrap_or_else(|| panic!("{err}"))
}

#[tokio::test]
async fn tar_xz_is_extracted_with_its_links_and_modes() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("mysql.tar.xz");
    let root = "mysql-8.4.13-linux-glibc2.28-x86_64-minimal";
    let p = |rest: &str| format!("{root}/{rest}");
    let (bin, lib, so, so3, link) = (
        p("bin/mysqld"),
        p("lib/private/libssl.so.3"),
        p("lib/private/libssl.so"),
        p("lib/libcrypto.so"),
        p("share/notes.txt"),
    );
    let tar = tar_bytes(&[
        Entry::Dir(&format!("{root}/")),
        Entry::File(&bin, b"\x7fELF mysqld", 0o755),
        Entry::File(&lib, b"ssl library", 0o644),
        // Same directory, then a chain through another directory.
        Entry::Symlink(&so, "libssl.so.3"),
        Entry::Symlink(&so3, "private/libssl.so"),
        Entry::File(&link, b"notes", 0o644),
    ]);
    fs::write(&archive, xz(&tar)).unwrap();

    let target = dir.path().join("components").join("mysql-8-4");
    let op = Operation::new("component-test")
        .step(Extract::new("extract", &archive, &target).strip(Strip::SingleRootDir));
    assert_eq!(
        run(op, &dir.path().join("state")).await.unwrap(),
        Outcome::Completed
    );

    assert_eq!(
        fs::read(target.join("bin/mysqld")).unwrap(),
        b"\x7fELF mysqld"
    );
    assert_eq!(
        fs::read(target.join("lib/private/libssl.so")).unwrap(),
        b"ssl library"
    );
    assert_eq!(
        fs::read(target.join("lib/libcrypto.so")).unwrap(),
        b"ssl library"
    );
    // Hard links, never symbolic ones: nothing can point outside the target.
    assert!(!fs::symlink_metadata(target.join("lib/libcrypto.so"))
        .unwrap()
        .is_symlink());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &str| fs::metadata(target.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("bin/mysqld"), 0o755);
        assert_eq!(mode("share/notes.txt"), 0o644);
    }
}

#[tokio::test]
async fn tar_xz_links_leaving_the_archive_are_refused() {
    for (link, target) in [
        ("r/lib/evil", "../../../etc/passwd"),
        ("r/lib/evil", "/etc/passwd"),
        ("r/lib/evil", "missing.so"),
        ("r/lib/evil", "../lib"), // a directory, not a file of the archive
        ("r/lib/evil", "evil"),   // itself
        ("r/lib/evil", r"..\..\..\x"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.tar.xz");
        let tar = tar_bytes(&[
            Entry::File("r/lib/ok.so", b"ok", 0o644),
            Entry::Symlink(link, target),
        ]);
        fs::write(&archive, xz(&tar)).unwrap();
        let out = dir.path().join("out").join("target");
        let op = Operation::new("evil")
            .step(Extract::new("extract", &archive, &out).strip(Strip::SingleRootDir));
        let err = run(op, &dir.path().join("state")).await.unwrap_err();
        assert!(
            matches!(extract_error(&err), ExtractError::UnsafeEntry(_)),
            "{target}: {err}"
        );
        assert!(!out.exists(), "{target}: target created");
    }
}

#[tokio::test]
async fn tar_xz_without_a_single_root_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("two.tar.xz");
    let tar = tar_bytes(&[
        Entry::File("a/x", b"1", 0o644),
        Entry::File("b/y", b"2", 0o644),
    ]);
    fs::write(&archive, xz(&tar)).unwrap();
    let op = Operation::new("two")
        .step(Extract::new("extract", &archive, dir.path().join("t")).strip(Strip::SingleRootDir));
    let err = run(op, &dir.path().join("state")).await.unwrap_err();
    assert!(
        matches!(extract_error(&err), ExtractError::NoSingleRoot),
        "{err}"
    );
}

#[tokio::test]
async fn package_files_complete_an_extracted_component() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("mysql.tar.xz");
    fs::write(
        &archive,
        xz(&tar_bytes(&[Entry::File("m/bin/mysql", b"client", 0o755)])),
    )
    .unwrap();
    let package = dir.path().join("libncurses6.deb");
    fs::write(
        &package,
        deb(&[
            Entry::Dir("./"),
            Entry::Dir("./lib/x86_64-linux-gnu/"),
            Entry::File(
                "./lib/x86_64-linux-gnu/libncurses.so.6.4",
                b"ncurses",
                0o644,
            ),
            Entry::Symlink(
                "./lib/x86_64-linux-gnu/libncurses.so.6",
                "libncurses.so.6.4",
            ),
            Entry::File("./usr/share/doc/copyright", b"license", 0o644),
        ]),
    )
    .unwrap();
    let target = dir.path().join("mysql-8-4");
    let files = BTreeMap::from([(
        "lib/x86_64-linux-gnu/libncurses.so.6.4".to_owned(),
        "lib/private/libncurses.so.6".to_owned(),
    )]);
    let op = Operation::new("component-test")
        .step(Extract::new("extract", &archive, &target).strip(Strip::SingleRootDir))
        .step(PackageFiles::new(
            "extra-0",
            &package,
            &target,
            files.clone(),
        ));
    assert_eq!(
        run(op, &dir.path().join("state")).await.unwrap(),
        Outcome::Completed
    );
    assert_eq!(
        fs::read(target.join("lib/private/libncurses.so.6")).unwrap(),
        b"ncurses"
    );
    assert_eq!(fs::read(target.join("bin/mysql")).unwrap(), b"client");
    assert!(!target.join("usr").exists());

    // A file the package lacks fails the operation: the component is rolled back.
    let missing = BTreeMap::from([(
        "lib/libtinfo.so.6".to_owned(),
        "lib/private/libtinfo.so.6".to_owned(),
    )]);
    let other = dir.path().join("other");
    let op = Operation::new("component-other")
        .step(Extract::new("extract", &archive, &other).strip(Strip::SingleRootDir))
        .step(PackageFiles::new("extra-0", &package, &other, missing));
    let err = run(op, &dir.path().join("state")).await.unwrap_err();
    assert!(
        err.to_string().contains("libtinfo")
            || matches!(extract_error(&err), ExtractError::Package { .. }),
        "{err}"
    );
    assert!(!other.exists(), "rolled back");
}
