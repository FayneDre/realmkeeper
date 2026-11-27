use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{Step, StepContext, StepFuture};

/// `package-files`: copies files out of a downloaded Debian package into an
/// unpacked component (libraries some Linux systems lack, ADR 0031).
///
/// It runs right after the component's `extract`, inside the same
/// operation: `undo` has nothing of its own to restore, since undoing that
/// `extract` puts the whole previous directory back. Each file is written
/// under a temporary name then renamed, so a rerun just writes it again.
pub struct PackageFiles {
    id: String,
    package: PathBuf,
    target: PathBuf,
    files: BTreeMap<String, String>,
}

impl PackageFiles {
    /// `files`: path in the package (no leading `./`) -> destination
    /// relative to `target`.
    pub fn new(
        id: impl Into<String>,
        package: impl Into<PathBuf>,
        target: impl Into<PathBuf>,
        files: BTreeMap<String, String>,
    ) -> Self {
        Self {
            id: id.into(),
            package: package.into(),
            target: target.into(),
            files,
        }
    }
}

impl<C: Sync> Step<C> for PackageFiles {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "package-files"
    }

    fn fingerprint(&self) -> String {
        format!(
            "{} -> {} {:?}",
            self.package.display(),
            self.target.display(),
            self.files
        )
    }

    fn run<'a>(&'a self, _cx: &'a StepContext<'a, C>) -> StepFuture<'a> {
        Box::pin(async move {
            let (package, target, files) = (
                self.package.clone(),
                self.target.clone(),
                self.files.clone(),
            );
            tokio::task::spawn_blocking(move || {
                super::tarball::unpack_package_files(&package, &target, &files)
            })
            .await
            .map_err(|e| format!("package thread failed: {e}"))??;
            Ok(())
        })
    }

    fn undo<'a>(&'a self, _cx: &'a StepContext<'a, C>) -> StepFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}
