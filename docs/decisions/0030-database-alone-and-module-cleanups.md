# 0030. Database started alone, and module cleanups run by hand

- Status: Accepted
- Date: 2026-11-27

## Context

Players open their realm's databases with a SQL tool (HeidiSQL) to look at
or fix data: a realm's MySQL only ran with its servers (ADR 0011), and
starting it by hand meant knowing `my.ini`, `client.cnf` and `mysqladmin`.
Changing data while the worldserver runs is unsafe: it keeps the world,
auctions and characters in memory and saves over them.

mod-ah-bot (pinned `c11d831`) never removes auctions above its target: at
each cycle its seller returns at once when the house holds at least
`maxitems` auctions (`AuctionHouseBot.cpp` `Sell`, `nbOfAuctions >=
maxTotalItems`), and it fills qualities in order (grey, white, green, then
blue and purple). A player who lowered the target from 20 000 to 10 000 kept
15 000 white and green auctions per house and no rare ones until they
expired. Auctions are core data: `acore_characters.auctionhouse`, one row per
auction, its item in `item_instance` (`auctionhouse.itemguid` ->
`item_instance.guid`, fork base SQL), a player database (`playerData`).

## Decision

1. **Database alone.** `rk_runtime::Server::hold_database` starts (or
   adopts) the realm's MySQL and returns a `DatabaseHold` holding the
   instance lock; `release_database` stops it cleanly. `Server::take_over`
   gives a hold to the next start, which takes its lock over (no gap where
   another process gets in). Stopping the servers stops everything, the
   database included.
2. **App.** `AppState.databases` keeps the holds: `start_database`,
   `stop_database` (refused with `database.serversRunning` while the servers
   run; adopts a database left running by a launcher that stopped
   abruptly), `database_status` (ping), `database_connection` (host
   `127.0.0.1`, the realm's port, the launcher's MySQL account from
   `secrets.json`). Start servers and Play take the hold over. Changes (the
   `operation` helper) are refused with `database.runningAlone`; account
   work borrows the running database (ADR 0011); a manual backup dumps it
   as it runs. Quitting, Windows shutdown and the shutdown block count held
   databases and stop them cleanly. Home: the row (named "MySQL") holds
   two icon buttons, Connection (greyed while down) and Start / Stop; the
   refusal is a toast. `stop_database` on a database already down does
   nothing (adopting would start it). CLI: `rk db start <id>` (foreground) and `rk db stop <id>`.
3. **Module cleanups.** A module manifest may declare `maintenance`
   actions: `{id, label, help?, warning, clear: {database, table, with:
   [{table, column, from}]}}`. A cleanup empties `table` of the emulator
   database `database`, after deleting the rows of each `with` table whose
   `column` is in the cleared rows' `from` column. Never free SQL: plain
   names only (schema, catalog validation and `ClearTable::sql`), no table
   alias in the `DELETE` (MySQL 8.4 refuses one without a default database,
   `ERROR 1046`). It may target a player database, unlike option table
   writes (ADR 0029), because the player asks for it explicitly.
4. **Running a cleanup** (`Launcher::run_maintenance`, app command
   `run_maintenance`, CLI `rk maintenance <id> <module> <action> --yes`):
   servers stopped (instance lock, or the app's hold of a database started
   alone); the module enabled; a dump of the player databases first
   (`before-maintenance-<action>-<secs>.sql`, automatic, pruned like the
   other `before-*`, kind `maintenance`); then one transaction (row count,
   dependents, table). The UI lists the actions under "Maintenance" in the
   module's options and asks for confirmation in a modal that shows the
   warning.
5. mod-ah-bot declares `clear-auctions`: `characters.auctionhouse` with
   `item_instance` (`guid` = `itemguid`). Its warning says that players'
   auctions go too, with their items and the gold of current bids. The bot
   fills the houses again at the next start.

## Consequences

- A SQL tool can be used safely: nothing else starts on the realm while
  its database runs alone, and the servers start on it without a restart.
- Cleanups stay declarative and reviewable in the catalog; a new kind of
  cleanup needs a new manifest shape (and a schema change), not SQL text.
- A cleanup on a player database is destructive: the dump made first is the
  way back (Backups tab, restore).
- Bids are not refunded: the warning says so.
