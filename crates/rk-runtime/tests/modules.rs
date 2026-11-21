//! Module set changes against the real catalog (no MySQL needed).

use std::path::PathBuf;

use rk_engine::install::{self, Layout, Network, NewInstance};
use rk_runtime::modules::set_modules;

fn catalog() -> rk_core::Catalog {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    rk_catalog::load_dir(&root).expect("catalog loads")
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn removing_a_world_changing_module_schedules_the_world_rebuild() {
    let catalog = catalog();
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    let request = NewInstance {
        id: "t".into(),
        emulator: "azerothcore".into(),
        flavor: None,
        modules: strings(&["mod-ah-bot", "mod-individual-progression"]),
        network: Network::Solo,
    };
    install::open_or_create(&catalog, &layout, &request).unwrap();

    // AHBot's SQL stays harmless while disabled: nothing to rebuild.
    let (record, _) = set_modules(&catalog, &layout, "t", &[], &strings(&["mod-ah-bot"])).unwrap();
    assert_eq!(record.modules, strings(&["mod-individual-progression"]));
    assert!(record.regenerate.is_empty());

    // Individual Progression changed `world` (and player databases, kept).
    let ip = strings(&["mod-individual-progression"]);
    let (record, old) = set_modules(&catalog, &layout, "t", &[], &ip).unwrap();
    assert!(record.modules.is_empty());
    assert_eq!(record.regenerate, strings(&["acore_world"]));
    assert_eq!(old.modules, ip);

    // The same command again (resuming) keeps the same plan.
    let (again, _) = set_modules(&catalog, &layout, "t", &[], &ip).unwrap();
    assert_eq!(again, record);
    let (saved, _) = install::open_existing(&layout, "t").unwrap();
    assert_eq!(saved, record);

    // Once applied, the request is cleared.
    let done = rk_runtime::modules::finished(&layout, &record).unwrap();
    assert!(done.regenerate.is_empty());

    // Mandatory modules are refused.
    let error = set_modules(&catalog, &layout, "t", &[], &strings(&["mod-playerbots"]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot be removed"), "{error}");
}

/// ADR 0027: optional SQL files follow the options; turning one off
/// schedules the world rebuild, turning one on does not.
#[test]
fn optional_sql_files_follow_the_options() {
    use rk_runtime::modules::set_options;

    let catalog = catalog();
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    let module = "mod-individual-progression";
    let request = NewInstance {
        id: "t".into(),
        emulator: "azerothcore".into(),
        flavor: None,
        modules: strings(&[module]),
        network: Network::Solo,
    };
    install::open_or_create(&catalog, &layout, &request).unwrap();
    let pairs = |items: &[(&str, &str)]| -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let sql_files = |record: &install::InstanceRecord| -> Vec<String> {
        let (_, secrets) = install::open_existing(&layout, "t").unwrap();
        let plan = install::instance_plan(&catalog, &layout, record, &secrets).unwrap();
        let step = plan
            .optional_sql
            .expect("the emulator declares optionalSql");
        let dir = step
            .dirs
            .iter()
            .find(|d| {
                d.target
                    .ends_with("modules/mod-individual-progression/data/sql/db-world/rk-optional")
            })
            .expect("one directory for world");
        dir.files.iter().map(|f| f.name.clone()).collect()
    };

    let (record, _) = install::open_existing(&layout, "t").unwrap();
    assert!(sql_files(&record).is_empty(), "defaults apply no file");

    let (record, _) = set_options(
        &catalog,
        &layout,
        "t",
        module,
        &pairs(&[("sql-phasing", "true"), ("sql-remove-heirlooms", "on")]),
    )
    .unwrap();
    assert!(
        record.regenerate.is_empty(),
        "turning files on rebuilds nothing"
    );
    assert_eq!(
        sql_files(&record),
        [
            "zz_optional_phasing.sql",
            "zz_optional_remove_heirlooms.sql"
        ]
    );
    let (_, secrets) = install::open_existing(&layout, "t").unwrap();
    let plan = install::instance_plan(&catalog, &layout, &record, &secrets).unwrap();
    let step = plan.optional_sql.unwrap();
    let source = &step
        .dirs
        .iter()
        .find(|d| !d.files.is_empty())
        .unwrap()
        .files[0]
        .source;
    assert!(source.ends_with(
        "sql/modules/mod-individual-progression/optional/sql/world/zz_optional_phasing.sql"
    ));

    let (record, _) = set_options(
        &catalog,
        &layout,
        "t",
        module,
        &pairs(&[("sql-phasing", "false")]),
    )
    .unwrap();
    assert_eq!(record.regenerate, strings(&["acore_world"]));
    assert_eq!(sql_files(&record), ["zz_optional_remove_heirlooms.sql"]);

    // Without the module, its directory is emptied.
    let (record, _) = set_modules(&catalog, &layout, "t", &[], &strings(&[module])).unwrap();
    assert!(sql_files(&record).is_empty());
}

/// ADR 0027: a failed change that was turning an optional SQL file on
/// leaves the world to rebuild (dbimport may have run part of the file).
#[test]
fn a_failed_optional_sql_change_rebuilds_the_world_next_time() {
    use rk_runtime::modules::{after_failure, set_options};

    let catalog = catalog();
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    let module = "mod-individual-progression";
    let request = NewInstance {
        id: "t".into(),
        emulator: "azerothcore".into(),
        flavor: None,
        modules: strings(&[module]),
        network: Network::Solo,
    };
    install::open_or_create(&catalog, &layout, &request).unwrap();
    let on = vec![("sql-phasing".to_owned(), "true".to_owned())];
    let (attempted, old) = set_options(&catalog, &layout, "t", module, &on).unwrap();
    let restored = after_failure(&catalog, &attempted, &old);
    assert!(restored.options.is_empty(), "the option is back off");
    assert_eq!(restored.regenerate, strings(&["acore_world"]));

    // A failed change that touched no SQL option restores the old record.
    let same = after_failure(&catalog, &old, &old);
    assert_eq!(same, old);
}
