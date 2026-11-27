//! Cleanups a module offers, run by hand on a stopped realm (ADR 0030):
//! the player data is dumped first, then one transaction empties a table.

use super::*;

/// What a cleanup did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceOutcome {
    /// Rows of the cleared table that were deleted.
    pub removed: u64,
    /// The backup made just before (`backups/<id>/`).
    pub backup: String,
}

impl Launcher {
    /// Runs the cleanup `action` of `module` on a realm whose servers are
    /// stopped. `held`: the caller holds the realm with its database up
    /// (started alone); otherwise the realm is locked and its database
    /// started for the work, then stopped. Refused while the servers run
    /// or a change works (instance lock).
    pub async fn run_maintenance(
        &self,
        id: &str,
        module: &str,
        action: &str,
        held: bool,
    ) -> Result<MaintenanceOutcome> {
        let (record, _) = install::open_existing(&self.layout, id)?;
        let unknown = |what: String| LauncherError::Plan(install::PlanError::Unknown(what));
        if !rk_runtime::modules::enabled_modules(&self.catalog, &record)?
            .iter()
            .any(|m| m == module)
        {
            return Err(unknown(format!("module '{module}' on realm '{id}'")));
        }
        let manifest = self
            .catalog
            .module(&record.emulator, module)
            .ok_or_else(|| unknown(format!("module '{module}'")))?;
        let clear = &manifest
            .maintenance
            .iter()
            .find(|a| a.id == action)
            .ok_or_else(|| unknown(format!("cleanup '{action}' of '{module}'")))?
            .clear;
        let database = self
            .emulator(&record.emulator)?
            .databases
            .iter()
            .find(|d| d.id == clear.database)
            .map(|d| d.name.clone())
            .ok_or_else(|| unknown(format!("database '{}'", clear.database)))?;
        let sql = clear
            .sql(&database)
            .ok_or_else(|| unknown(format!("cleanup '{action}': table names")))?;

        let _lock = if held {
            None
        } else {
            Some(install::lock_instance(&self.layout, id)?)
        };
        let db = install::database(&self.catalog, &self.layout, &record)?;
        if !held {
            db.start().await?;
        }
        let done = self.clear_after_backup(id, action, &db, &sql).await;
        if !held {
            let stopped = db.stop().await;
            let outcome = done?;
            stopped?;
            return Ok(outcome);
        }
        done
    }

    async fn clear_after_backup(
        &self,
        id: &str,
        action: &str,
        db: &rk_db::Instance,
        sql: &str,
    ) -> Result<MaintenanceOutcome> {
        let name = format!("before-maintenance-{action}-{}.sql", unix_secs());
        let backup = self.dump_player_data(id, name, true).await?;
        let output = db.execute(sql).await?;
        let removed = output
            .lines()
            .next()
            .and_then(|l| l.trim().parse().ok())
            .unwrap_or(0);
        let _ = backups::prune(&self.layout.backups(id));
        Ok(MaintenanceOutcome {
            removed,
            backup: backup.name,
        })
    }
}
