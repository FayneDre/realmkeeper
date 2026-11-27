use std::collections::BTreeSet;
use std::fmt;

use rk_core::manifest::{Activation, ModuleOption, PostInstallStep};
use rk_core::Catalog;

/// A cross-manifest consistency problem. `location` is `emulator/kind/id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub location: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

/// `relativePath` of `module.schema.json`: `/`-separated segments of ASCII
/// letters, digits, `.`, `_` or `-`, never `.` or `..`.
fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

/// Placeholders each party command may use, and must use.
const PARTY_COMMANDS: [(&str, &[&str]); 3] = [
    ("add", &["player", "class", "spec", "level"]),
    ("remove", &["player"]),
    ("list", &["player"]),
];

/// A party (ADR 0025): commands with exactly their placeholders, unique
/// classes, and every class with a spec, each spec number once.
fn party_issues(party: &rk_core::manifest::Party) -> Vec<String> {
    let mut issues = Vec::new();
    for (name, wanted) in PARTY_COMMANDS {
        let command = match name {
            "add" => &party.commands.add,
            "remove" => &party.commands.remove,
            _ => &party.commands.list,
        };
        let used: BTreeSet<&str> = rk_core::console::placeholders(command)
            .into_iter()
            .collect();
        let wanted: BTreeSet<&str> = wanted.iter().copied().collect();
        if used != wanted {
            issues.push(format!(
                "party command '{name}' must use exactly {{{}}}",
                wanted.into_iter().collect::<Vec<_>>().join("}, {")
            ));
        }
    }
    let mut ids = BTreeSet::new();
    let mut tokens = BTreeSet::new();
    let mut class_ids = BTreeSet::new();
    for class in &party.classes {
        if !ids.insert(&class.id)
            || !tokens.insert(&class.token)
            || !class_ids.insert(class.class_id)
        {
            issues.push(format!("party class '{}' declared twice", class.id));
        }
        let mut numbers = BTreeSet::new();
        for spec in &class.specs {
            if !numbers.insert(spec.number) {
                issues.push(format!(
                    "party class '{}' declares spec {} twice",
                    class.id, spec.number
                ));
            }
        }
    }
    issues
}

/// Checks references between manifests. Returns every issue found (empty = valid).
pub fn validate(catalog: &Catalog) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut push = |location: String, message: String| issues.push(Issue { location, message });

    for (emu_id, entry) in &catalog.emulators {
        let emu = &entry.manifest;
        let here = format!("{emu_id}/emulator");

        if !catalog.clients.contains_key(&emu.client) {
            push(here.clone(), format!("unknown client '{}'", emu.client));
        }
        for component in &emu.components {
            if !catalog.components.contains_key(&component.id) {
                push(
                    here.clone(),
                    format!("unknown component '{}'", component.id),
                );
            }
        }
        if !entry.flavors.contains_key(&emu.default_flavor) {
            push(
                here.clone(),
                format!("unknown default flavor '{}'", emu.default_flavor),
            );
        }
        // Server data (ADR 0006, 0024): downloaded for the client, or
        // extracted from it with known placeholders, in a locale it has.
        match &emu.data_extraction {
            None => {
                if catalog
                    .clients
                    .get(&emu.client)
                    .is_some_and(|c| c.server_data.is_none())
                {
                    push(
                        here.clone(),
                        format!(
                            "no server data: client '{}' has no serverData and no dataExtraction is declared",
                            emu.client
                        ),
                    );
                }
            }
            Some(extraction) => {
                if let Some(client) = catalog.clients.get(&emu.client) {
                    if !client.locales.contains(&extraction.locale) {
                        push(
                            here.clone(),
                            format!(
                                "data extraction needs locale '{}', which client '{}' does not list",
                                extraction.locale, emu.client
                            ),
                        );
                    }
                }
                let mut step_ids = BTreeSet::new();
                for step in &extraction.steps {
                    if !step_ids.insert(&step.id) {
                        push(
                            here.clone(),
                            format!("extraction step '{}' declared twice", step.id),
                        );
                    }
                    for arg in &step.args {
                        match rk_core::template::placeholders(arg) {
                            Ok(names) => {
                                for name in names {
                                    if !rk_core::manifest::EXTRACTION_PLACEHOLDERS.contains(&name) {
                                        push(
                                            here.clone(),
                                            format!(
                                                "extraction step '{}' uses unknown '${{{name}}}'",
                                                step.id
                                            ),
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                push(here.clone(), format!("extraction step '{}': {e}", step.id))
                            }
                        }
                    }
                }
            }
        }

        // GM panel: every placeholder is a parameter, ids are unique, and
        // start milestones only move forward.
        let mut action_ids = std::collections::BTreeSet::new();
        for action in &emu.console_actions {
            if !action_ids.insert(&action.id) {
                push(
                    here.clone(),
                    format!("console action '{}' declared twice", action.id),
                );
            }
            for name in rk_core::console::placeholders(&action.command) {
                if !action.params.iter().any(|p| p.id == name) {
                    push(
                        here.clone(),
                        format!(
                            "console action '{}' uses undeclared '{{{name}}}'",
                            action.id
                        ),
                    );
                }
            }
            for param in &action.params {
                if let rk_core::manifest::ConsoleParamKind::Number { min, max, default } =
                    &param.kind
                {
                    if min > max || default.is_some_and(|d| d < *min || d > *max) {
                        push(
                            here.clone(),
                            format!(
                                "console action '{}': bad range for '{}'",
                                action.id, param.id
                            ),
                        );
                    }
                }
            }
        }
        for process in &emu.processes {
            if process
                .startup
                .windows(2)
                .any(|w| w[1].progress < w[0].progress)
            {
                push(
                    here.clone(),
                    format!("process '{}': startup milestones go backwards", process.id),
                );
            }
            if let rk_core::manifest::Readiness::Tcp { port } = &process.readiness {
                if !emu.ports.contains_key(port) {
                    push(
                        here.clone(),
                        format!("process '{}' waits on undeclared port '{port}'", process.id),
                    );
                }
            }
        }

        // Optional module SQL (ADR 0027): both paths stay in the SQL tree,
        // one directory per module and database.
        if let Some(spec) = &emu.instance.optional_sql {
            let source = spec.source_path("m", "f.sql");
            let target = spec.target_dir("m", "d");
            if !spec.source.contains("{file}") || !is_safe_relative(&source) {
                push(
                    here.clone(),
                    "instance.optionalSql.source: a relative path with '{file}'".into(),
                );
            }
            if !spec.target.contains("{module}")
                || !spec.target.contains("{database}")
                || !is_safe_relative(&target)
            {
                push(
                    here.clone(),
                    "instance.optionalSql.target: a relative path with '{module}' and '{database}'"
                        .into(),
                );
            }
        }

        // Realm settings (ADR 0017): core config files only, never a key the
        // launcher imposes, no assets, unique ids.
        if entry.modules.contains_key(rk_core::manifest::REALM_OPTIONS) {
            push(
                here.clone(),
                format!(
                    "module id '{}' is reserved for the realm settings",
                    rk_core::manifest::REALM_OPTIONS
                ),
            );
        }
        let mut setting_ids = BTreeSet::new();
        for option in emu.settings.iter().flat_map(|s| &s.options) {
            let id = option.id();
            if !setting_ids.insert(id) {
                push(here.clone(), format!("duplicate realm setting '{id}'"));
            }
            let mut targets: Vec<(&str, &str)> = Vec::new();
            match option {
                rk_core::manifest::ModuleOption::Boolean { target, .. }
                | rk_core::manifest::ModuleOption::Integer { target, .. } => {
                    match target.config() {
                        Some(target) => targets.push((&target.file, &target.key)),
                        None => push(
                            here.clone(),
                            format!("realm setting '{id}': only module options can write tables"),
                        ),
                    }
                }
                rk_core::manifest::ModuleOption::Number { target, .. } => {
                    targets.push((&target.file, &target.key))
                }
                rk_core::manifest::ModuleOption::Sql { .. } => push(
                    here.clone(),
                    format!("realm setting '{id}': only module options can apply SQL files"),
                ),
                rk_core::manifest::ModuleOption::Choice { choices, .. } => {
                    for choice in choices {
                        if !choice.assets.is_empty() {
                            push(
                                here.clone(),
                                format!("realm setting '{id}': choices cannot install assets"),
                            );
                        }
                        if !choice.tables.is_empty() {
                            push(
                                here.clone(),
                                format!(
                                    "realm setting '{id}': only module options can write tables"
                                ),
                            );
                        }
                        for (file, keys) in &choice.config {
                            targets.extend(keys.keys().map(|k| (file.as_str(), k.as_str())));
                        }
                    }
                }
            }
            for (file, key) in targets {
                match emu.instance.config.get(file) {
                    None => push(
                        here.clone(),
                        format!("realm setting '{id}': '{file}' is not a core config file"),
                    ),
                    Some(locked) if locked.contains_key(key) => push(
                        here.clone(),
                        format!("realm setting '{id}': '{key}' is imposed by the launcher"),
                    ),
                    Some(_) => {}
                }
            }
        }

        // Instance placeholders (ADR 0007): a closed list, plus declared ports.
        let mut check_template = |text: &str, allowed: &dyn Fn(&str) -> bool, what: &str| {
            match rk_core::template::placeholders(text) {
                Err(e) => push(here.clone(), format!("{what}: {e}")),
                Ok(names) => {
                    for name in names.into_iter().filter(|n| !allowed(n)) {
                        push(
                            here.clone(),
                            format!("{what}: unknown placeholder '${{{name}}}'"),
                        );
                    }
                }
            }
        };
        let instance_var = |name: &str| {
            rk_core::template::INSTANCE_VARIABLES.contains(&name)
                || name
                    .strip_prefix("port.")
                    .is_some_and(|port| emu.ports.contains_key(port))
        };
        let connection_var = |name: &str| rk_core::template::CONNECTION_VARIABLES.contains(&name);
        check_template(
            &emu.instance.connection,
            &connection_var,
            "instance.connection",
        );
        for (file, keys) in &emu.instance.config {
            for (key, value) in keys {
                if let rk_core::manifest::ConfigValue::Text(text) = value {
                    check_template(
                        text,
                        &instance_var,
                        &format!("instance.config {file} {key}"),
                    );
                }
            }
        }

        let mut database_ids: BTreeSet<&str> =
            emu.databases.iter().map(|d| d.id.as_str()).collect();
        for module in entry.modules.values() {
            database_ids.extend(
                module
                    .database
                    .extra_databases
                    .iter()
                    .map(|d| d.id.as_str()),
            );
        }

        for flavor in entry.flavors.values() {
            let here = format!("{emu_id}/flavor/{}", flavor.id);
            for id in &flavor.required_modules {
                match entry.modules.get(id) {
                    None => push(here.clone(), format!("unknown required module '{id}'")),
                    Some(m) if !m.compatible_flavors.contains(&flavor.id) => push(
                        here.clone(),
                        format!("required module '{id}' does not list this flavor as compatible"),
                    ),
                    Some(m) if m.removable => push(
                        here.clone(),
                        format!("required module '{id}' must be \"removable\": false"),
                    ),
                    Some(_) => {}
                }
            }
            // Whatever a required module pulls in is mandatory too (the UI
            // shows it as included), so it must not be removable either.
            for id in catalog.mandatory_modules(emu_id, &flavor.id) {
                if flavor.required_modules.contains(&id) {
                    continue;
                }
                if let Some(m) = entry.modules.get(&id) {
                    if m.removable {
                        push(
                            here.clone(),
                            format!(
                                "module '{id}' is required through a required module and must be \"removable\": false"
                            ),
                        );
                    }
                }
            }
        }

        // At most one module offers bot parties (ADR 0025): the launcher has
        // one Party page per realm.
        let party_modules: Vec<&str> = entry
            .modules
            .values()
            .filter(|m| m.party.is_some())
            .map(|m| m.id.as_str())
            .collect();
        if party_modules.len() > 1 {
            push(
                here.clone(),
                format!(
                    "several modules offer a party: {}",
                    party_modules.join(", ")
                ),
            );
        }

        for module in entry.modules.values() {
            let here = format!("{emu_id}/module/{}", module.id);
            if let Some(party) = &module.party {
                for issue in party_issues(party) {
                    push(here.clone(), issue);
                }
            }
            for issue in maintenance_issues(&entry.manifest, module) {
                push(here.clone(), issue);
            }
            for flavor in &module.compatible_flavors {
                if !entry.flavors.contains_key(flavor) {
                    push(
                        here.clone(),
                        format!("unknown compatible flavor '{flavor}'"),
                    );
                }
            }
            for id in module.requires.iter().chain(&module.conflicts) {
                if !entry.modules.contains_key(id) {
                    push(here.clone(), format!("references unknown module '{id}'"));
                }
                if id == &module.id {
                    push(here.clone(), "references itself".into());
                }
            }
            if matches!(module.activation, Activation::Always) && module.removable {
                push(
                    here.clone(),
                    "activation 'always' requires \"removable\": false".into(),
                );
            }
            for db in &module.database.touches {
                if !database_ids.contains(db.as_str()) {
                    push(here.clone(), format!("touches unknown database '{db}'"));
                }
            }

            let asset_ids: BTreeSet<&str> = module.assets.iter().map(|a| a.id.as_str()).collect();
            // Asset paths end up joined to the client and data folders: the
            // schema's rule again, so no manifest can write outside them.
            for asset in &module.assets {
                if asset.source.url(&module.source).is_none() {
                    push(
                        here.clone(),
                        format!(
                            "asset '{}': source needs either 'path' or a GitHub 'repository' and its 'commit'",
                            asset.id
                        ),
                    );
                }
                if let Some(commit) = &asset.source.commit {
                    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                        push(
                            here.clone(),
                            format!("asset '{}': commit must be 40 hex digits", asset.id),
                        );
                    }
                }
                let files = asset
                    .client
                    .iter()
                    .chain(&asset.server)
                    .chain(&asset.addons);
                let paths = asset
                    .source
                    .path
                    .iter()
                    .map(String::as_str)
                    .chain(files.flat_map(|f| [f.from.as_str(), f.to.as_str()]));
                for path in paths {
                    if !is_safe_relative(path) {
                        push(
                            here.clone(),
                            format!("asset '{}': unsafe path '{path}'", asset.id),
                        );
                    }
                }
            }
            let mut option_ids = BTreeSet::new();
            // The migration tool orders files by name and refuses duplicates.
            let mut sql_names = BTreeSet::new();
            for option in &module.options {
                if !option_ids.insert(option.id()) {
                    push(here.clone(), format!("duplicate option '{}'", option.id()));
                }
                // Table targets (ADR 0029): a content database the module
                // touches, plain identifiers (they end up in SQL).
                for issue in table_issues(emu, module, option) {
                    push(here.clone(), issue);
                }
                if let ModuleOption::Choice {
                    id,
                    default,
                    choices,
                    ..
                } = option
                {
                    if !choices.iter().any(|c| &c.id == default) {
                        push(
                            here.clone(),
                            format!("option '{id}': default '{default}' is not a choice"),
                        );
                    }
                    for choice in choices {
                        for asset in &choice.assets {
                            if !asset_ids.contains(asset.as_str()) {
                                push(
                                    here.clone(),
                                    format!("option '{id}': unknown asset '{asset}'"),
                                );
                            }
                        }
                    }
                }
                // Optional SQL (ADR 0027): a file of the module, for a
                // content database it touches (turning it off rebuilds that
                // database; player data is never rebuilt).
                if let ModuleOption::Sql {
                    id,
                    requires: Some(requirement),
                    default,
                    ..
                } = option
                {
                    for issue in requirement_issues(module, id, requirement, *default) {
                        push(here.clone(), issue);
                    }
                }
                if let ModuleOption::Sql {
                    id, database, file, ..
                } = option
                {
                    if emu.instance.optional_sql.is_none() || emu.instance.db_import.is_none() {
                        push(
                            here.clone(),
                            format!(
                                "option '{id}': the emulator declares no 'instance.optionalSql' and 'dbImport'"
                            ),
                        );
                    }
                    if !module.database.touches.contains(database) {
                        push(
                            here.clone(),
                            format!("option '{id}': database '{database}' is not in 'touches'"),
                        );
                    }
                    if emu
                        .databases
                        .iter()
                        .any(|d| &d.id == database && d.player_data)
                    {
                        push(
                            here.clone(),
                            format!("option '{id}': '{database}' holds player data"),
                        );
                    }
                    let name = file.rsplit('/').next().unwrap_or(file);
                    if !sql_names.insert(name) {
                        push(
                            here.clone(),
                            format!("option '{id}': another option uses a file named '{name}'"),
                        );
                    }
                    if !is_safe_relative(file) || !file.ends_with(".sql") {
                        push(
                            here.clone(),
                            format!("option '{id}': '{file}' is not a relative .sql path"),
                        );
                    }
                }
                if let ModuleOption::Integer {
                    id,
                    default,
                    min,
                    max,
                    ..
                } = option
                {
                    if min.is_some_and(|m| *default < m) || max.is_some_and(|m| *default > m) {
                        push(
                            here.clone(),
                            format!("option '{id}': default {default} is out of range"),
                        );
                    }
                }
            }

            // Option groups: known options, each in one group at most.
            let mut group_ids = BTreeSet::new();
            let mut grouped = BTreeSet::new();
            for group in &module.option_groups {
                if !group_ids.insert(group.id.as_str()) {
                    push(
                        here.clone(),
                        format!("duplicate option group '{}'", group.id),
                    );
                }
                for option in &group.options {
                    if !option_ids.contains(option.as_str()) {
                        push(
                            here.clone(),
                            format!("option group '{}': unknown option '{option}'", group.id),
                        );
                    }
                    if !grouped.insert(option.as_str()) {
                        push(
                            here.clone(),
                            format!("option '{option}' is in several groups"),
                        );
                    }
                }
            }

            let mut bindings = BTreeSet::new();
            for step in &module.post_install {
                match step {
                    PostInstallStep::CreateAccount { binding, .. } => {
                        bindings.insert(binding.as_str());
                    }
                    PostInstallStep::CreateCharacter {
                        binding, account, ..
                    } => {
                        if !bindings.contains(account.as_str()) {
                            push(
                                here.clone(),
                                format!("create-character uses unbound account '{account}'"),
                            );
                        }
                        bindings.insert(binding.as_str());
                    }
                    PostInstallStep::SetConfig { values } => {
                        for value in values.values().flat_map(|keys| keys.values()) {
                            if let rk_core::manifest::ConfigValue::Text(text) = value {
                                for reference in placeholders(text) {
                                    let name = reference.split('.').next().unwrap_or_default();
                                    if !bindings.contains(name) {
                                        push(
                                            here.clone(),
                                            format!("set-config uses unbound '${{{reference}}}'"),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    issues
}

/// Extracts `name.field` from every `${name.field}` in `text`.
fn placeholders(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                out.push(&after[..end]);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// A requirement names another option of the same module and values it can
/// take; an option on by default must have its requirement met by default.
fn requirement_issues(
    module: &rk_core::manifest::ModuleManifest,
    id: &str,
    requirement: &rk_core::manifest::OptionRequirement,
    default: bool,
) -> Vec<String> {
    use rk_core::manifest::ModuleOption;
    let mut issues = Vec::new();
    let required = &requirement.option;
    let Some(target) = module.options.iter().find(|o| o.id() == required) else {
        return vec![format!(
            "option '{id}': requires unknown option '{required}'"
        )];
    };
    if required == id {
        issues.push(format!("option '{id}': requires itself"));
    }
    let allowed: Vec<String> = match target {
        ModuleOption::Choice { choices, .. } => choices.iter().map(|c| c.id.clone()).collect(),
        ModuleOption::Boolean { .. } | ModuleOption::Sql { .. } => {
            vec!["true".into(), "false".into()]
        }
        _ => {
            issues.push(format!(
                "option '{id}': can only require a choice or a boolean option"
            ));
            Vec::new()
        }
    };
    if requirement.values.is_empty() {
        issues.push(format!("option '{id}': requires no value of '{required}'"));
    }
    for value in &requirement.values {
        if !allowed.is_empty() && !allowed.contains(value) {
            issues.push(format!(
                "option '{id}': '{value}' is not a value of '{required}'"
            ));
        }
    }
    if default && !module.requirement_met(requirement, None) {
        issues.push(format!(
            "option '{id}': on by default while '{required}' is not"
        ));
    }
    issues
}

/// A plain SQL identifier: lower-case letters, digits and `_`.
fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Problems of the table writes an option declares (ADR 0029).
fn table_issues(
    emu: &rk_core::manifest::EmulatorManifest,
    module: &rk_core::manifest::ModuleManifest,
    option: &ModuleOption,
) -> Vec<String> {
    use rk_core::manifest::OptionTarget;
    let id = option.id();
    // (database, table, where columns, set columns)
    let mut writes: Vec<(&str, &str, Vec<&str>, Vec<&str>)> = Vec::new();
    match option {
        ModuleOption::Boolean {
            target: OptionTarget::Table(t),
            ..
        }
        | ModuleOption::Integer {
            target: OptionTarget::Table(t),
            ..
        } => writes.push((
            &t.database,
            &t.table,
            t.row.keys().map(String::as_str).collect(),
            vec![t.column.as_str()],
        )),
        ModuleOption::Choice { choices, .. } => {
            for change in choices.iter().flat_map(|c| &c.tables) {
                writes.push((
                    &change.database,
                    &change.table,
                    change.row.keys().map(String::as_str).collect(),
                    change.set.keys().map(String::as_str).collect(),
                ));
            }
        }
        _ => {}
    }
    let mut issues = Vec::new();
    for (database, table, row, set) in writes {
        if !module.database.touches.iter().any(|d| d == database) {
            issues.push(format!(
                "option '{id}': database '{database}' is not in 'touches'"
            ));
        }
        if emu
            .databases
            .iter()
            .any(|d| d.id == database && d.player_data)
        {
            issues.push(format!("option '{id}': '{database}' holds player data"));
        }
        if row.is_empty() || set.is_empty() {
            issues.push(format!(
                "option '{id}': a table write needs 'where' and a column to set"
            ));
        }
        for name in std::iter::once(table).chain(row).chain(set) {
            if !is_identifier(name) {
                issues.push(format!("option '{id}': '{name}' is not a plain SQL name"));
            }
        }
    }
    issues
}

/// Problems of the cleanups a module offers (ADR 0030).
fn maintenance_issues(
    emu: &rk_core::manifest::EmulatorManifest,
    module: &rk_core::manifest::ModuleManifest,
) -> Vec<String> {
    let mut issues = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for action in &module.maintenance {
        let id = &action.id;
        if !seen.insert(id.as_str()) {
            issues.push(format!("maintenance '{id}' is declared twice"));
        }
        if !emu.databases.iter().any(|d| d.id == action.clear.database) {
            issues.push(format!(
                "maintenance '{id}': unknown database '{}'",
                action.clear.database
            ));
        }
        for name in action.clear.names() {
            if !is_identifier(name) {
                issues.push(format!(
                    "maintenance '{id}': '{name}' is not a plain SQL name"
                ));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::placeholders;

    #[test]
    fn extracts_placeholders() {
        assert_eq!(placeholders("${a.id}-${b.guid}"), vec!["a.id", "b.guid"]);
        assert!(placeholders("plain").is_empty());
    }
}
