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

/// ADR 0027: a file that needs a client patch is refused without it, and
/// the patch cannot go while the file is on.
#[test]
fn optional_sql_requirements_are_enforced() {
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
    let poisons = ("sql-restore-rogue-poisons", "true");

    let error = set_options(&catalog, &layout, "t", module, &pairs(&[poisons]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("mana-costs"), "{error}");

    let (record, _) = set_options(
        &catalog,
        &layout,
        "t",
        module,
        &pairs(&[("mana-costs", "era-adjusted"), poisons]),
    )
    .unwrap();
    let manifest = &catalog.emulator("azerothcore").unwrap().modules[module];
    assert_eq!(
        manifest.selected_sql(record.options.get(module)),
        [(
            "world",
            "optional/sql/world/zz_optional_restore_rogue_poisons.sql"
        )]
    );

    // The patch cannot go alone...
    let error = set_options(
        &catalog,
        &layout,
        "t",
        module,
        &pairs(&[("mana-costs", "unchanged")]),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("sql-restore-rogue-poisons"), "{error}");

    // ...but together with the file, and the world is rebuilt without it.
    let (record, _) = set_options(
        &catalog,
        &layout,
        "t",
        module,
        &pairs(&[
            ("mana-costs", "unchanged"),
            ("sql-restore-rogue-poisons", "false"),
        ]),
    )
    .unwrap();
    assert!(manifest.selected_sql(record.options.get(module)).is_empty());
    assert_eq!(record.regenerate, strings(&["acore_world"]));
}

/// A realm taken back from another computer (ADR 0028) takes its modules
/// and options, with the rebuilds a removal or a turned-off file needs.
#[test]
fn another_computers_choices_replace_these() {
    use rk_core::manifest::OptionValue;
    use rk_runtime::modules::{set_choices, set_options};
    use std::collections::BTreeMap;

    let catalog = catalog();
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    let ip = "mod-individual-progression";
    let request = NewInstance {
        id: "t".into(),
        emulator: "azerothcore".into(),
        flavor: None,
        modules: strings(&[ip]),
        network: Network::Solo,
    };
    install::open_or_create(&catalog, &layout, &request).unwrap();
    let on = vec![("sql-vanilla-regen-values".to_string(), "true".to_string())];
    set_options(&catalog, &layout, "t", ip, &on).unwrap();

    // The other computer has AHBot instead, and a realm setting.
    let mut options = BTreeMap::new();
    options.insert(
        rk_core::manifest::REALM_OPTIONS.to_string(),
        BTreeMap::from([("max-level".to_string(), OptionValue::Integer(70))]),
    );
    let (record, old) =
        set_choices(&catalog, &layout, "t", &strings(&["mod-ah-bot"]), &options).unwrap();
    assert_eq!(old.modules, strings(&[ip]));
    assert_eq!(record.modules, strings(&["mod-ah-bot"]));
    assert_eq!(record.options, options);
    assert_eq!(record.regenerate, strings(&["acore_world"]));
    assert_eq!(install::open_existing(&layout, "t").unwrap().0, record);

    // A value this catalog does not offer is refused, nothing saved.
    let bad = BTreeMap::from([(
        "mod-ah-bot".to_string(),
        BTreeMap::from([("no-such-option".to_string(), OptionValue::Bool(true))]),
    )]);
    assert!(set_choices(&catalog, &layout, "t", &strings(&["mod-ah-bot"]), &bad).is_err());
    assert_eq!(install::open_existing(&layout, "t").unwrap().0, record);

    // Options the other computer kept for a module it turned off come
    // along, unchecked, like `set_modules` keeps them.
    let kept = BTreeMap::from([(
        "mod-transmog".to_string(),
        BTreeMap::from([("portable".to_string(), OptionValue::Bool(false))]),
    )]);
    let (record, _) =
        set_choices(&catalog, &layout, "t", &strings(&["mod-ah-bot"]), &kept).unwrap();
    assert_eq!(record.options, kept);
}
