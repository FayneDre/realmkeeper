//! `optional-sql` (ADR 0027): puts the optional SQL files the player turned
//! on where the migration tool reads module SQL, before it runs.
//!
//! Each module with `sql` options owns one directory per database in the
//! instance SQL tree (`instance.optionalSql.target`); the step leaves exactly
//! the chosen files in it, copied from the server artifact
//! (`instance.optionalSql.source`). Turning a file off removes it here; the
//! database it changed is rebuilt by the module operation (`regenerate`),
//! never "un-applied".
//!
//! Idempotent, so nothing to undo: the next operation syncs the directories
//! again from the record it runs with.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::{fsutil, Step, StepContext, StepFuture};

/// The launcher-owned directories of every module with `sql` options.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionalSql {
    pub dirs: Vec<OptionalSqlDir>,
}

/// One module's directory for one database, and the files it must hold.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionalSqlDir {
    pub target: PathBuf,
    pub files: Vec<OptionalSqlFile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionalSqlFile {
    /// In the instance SQL tree (extracted from the server artifact).
    pub source: PathBuf,
    /// File name in the target directory (the migration tool orders files
    /// by name).
    pub name: String,
}

impl OptionalSql {
    /// Makes every directory hold exactly its chosen files.
    pub fn sync(&self) -> Result<(), String> {
        for dir in &self.dirs {
            if dir.files.is_empty() && !dir.target.exists() {
                continue;
            }
            std::fs::create_dir_all(&dir.target)
                .map_err(|e| format!("{}: {e}", dir.target.display()))?;
            let wanted: BTreeSet<&str> = dir.files.iter().map(|f| f.name.as_str()).collect();
            let entries = std::fs::read_dir(&dir.target)
                .map_err(|e| format!("{}: {e}", dir.target.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("{}: {e}", dir.target.display()))?;
                let name = entry.file_name();
                if !wanted.contains(name.to_string_lossy().as_ref()) {
                    let path = entry.path();
                    let removed = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                    removed.map_err(|e| format!("{}: {e}", path.display()))?;
                }
            }
            for file in &dir.files {
                let bytes = std::fs::read(&file.source).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        format!(
                            "{} is missing from the server files of this realm \
                             (made by an older server build): update the realm",
                            file.source.display()
                        )
                    } else {
                        format!("{}: {e}", file.source.display())
                    }
                })?;
                let target = dir.target.join(&file.name);
                if std::fs::read(&target).ok().as_deref() != Some(bytes.as_slice()) {
                    fsutil::write_atomic(&target, &bytes).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }
}

impl<C: Sync> Step<C> for OptionalSql {
    fn id(&self) -> &str {
        "optional-sql"
    }
    fn kind(&self) -> &str {
        "optional-sql"
    }
    fn fingerprint(&self) -> String {
        format!("{:?}", self.dirs)
    }
    fn run<'a>(&'a self, _cx: &'a StepContext<'a, C>) -> StepFuture<'a> {
        Box::pin(async move { Ok(self.sync()?) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rk-optional-sql-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn leaves_exactly_the_chosen_files() {
        let root = temp();
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("zz_a.sql"), "a").unwrap();
        std::fs::write(source.join("zz_b.sql"), "b").unwrap();
        let target = root.join("target");
        let plan = |names: &[&str]| OptionalSql {
            dirs: vec![OptionalSqlDir {
                target: target.clone(),
                files: names
                    .iter()
                    .map(|n| OptionalSqlFile {
                        source: source.join(n),
                        name: (*n).to_owned(),
                    })
                    .collect(),
            }],
        };
        let listing = || {
            let mut names: Vec<String> = std::fs::read_dir(&target)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };

        // Nothing chosen and no directory yet: nothing created.
        plan(&[]).sync().unwrap();
        assert!(!target.exists());

        plan(&["zz_a.sql", "zz_b.sql"]).sync().unwrap();
        assert_eq!(listing(), ["zz_a.sql", "zz_b.sql"]);

        // A file turned off goes; an edited source is copied again.
        std::fs::write(source.join("zz_a.sql"), "a2").unwrap();
        plan(&["zz_a.sql"]).sync().unwrap();
        assert_eq!(listing(), ["zz_a.sql"]);
        assert_eq!(std::fs::read(target.join("zz_a.sql")).unwrap(), b"a2");

        // A file missing from an older server build: a clear error.
        let error = plan(&["zz_missing.sql"]).sync().unwrap_err();
        assert!(error.contains("update the realm"), "{error}");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
