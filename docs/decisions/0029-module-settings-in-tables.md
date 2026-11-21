# 0029. Module settings kept in database tables

- Status: Accepted
- Date: 2026-11-21

## Context

Module options write config keys (ADR 0004, 0013). Some modules keep their
settings in a table of a content database instead. mod-ah-bot (pinned
`c11d831`) reads, at startup, one row per auction house (2 Alliance, 6
Horde, 7 neutral) of `acore_world.mod_auctionhousebot`
(`AuctionHouseBotConfig.cpp` `InitializeFromSql`): `minitems` / `maxitems`,
the share of each quality of trade goods and items (`percent<quality>tradegoods`,
`percent<quality>items`, 14 columns that must sum to 100, else the module
falls back to its own defaults), buyout prices (`minprice<quality>` /
`maxprice<quality>`, percent of the item's vendor price) and more. Its SQL
creates the rows with `maxitems = 250`: with Playerbots' thousands of bots
the auction houses look empty (the maintainer's realm had 741 auctions in
all). The seller fills a house up to `maxitems` (`minitems` only sets a
flag), `ItemsPerCycle` at a time.

The only other way to change these values is the GM console
(`.ahbotoptions maxitems <house> <n>`, `Console::Yes`), and a world
rebuild (ADR 0012) resets them to the module's SQL.

## Decision

1. A boolean or integer option's `target` is either a config key
   (`{file, key}`) or a column of one row of a content database table:
   `{database, table, column, where: {column: integer}}`.
2. A choice may set columns too: `tables: [{database, table, where, set:
   {column: integer}}]`, next to its `config` and `assets`.
3. Rules (catalog validation): the database is one the module `touches`
   and holds no player data; table and column names are plain SQL names
   (`^[a-z_][a-z0-9_]{0,63}$`, checked again before use); values are
   integers. Realm settings cannot write tables.
4. Before each start, after the driver's `prepare_start`, `rk-runtime`
   runs one `UPDATE <db>.<table> SET ... WHERE ...` per write of the
   enabled modules' effective values (defaults included), in manifest
   order. The values therefore survive world rebuilds, and the launcher's
   values win over console changes at the next start. A missing table
   fails the start (the module's SQL is applied by then).
5. mod-ah-bot options: auctions per house (`maxitems`, default 250 as
   upstream, 0-20 000), what is sold (`quality-mix`: as upstream, mostly
   trade goods, more gear, more rare and epic items; each sums to 100) and
   prices (`prices`: ×0.5, as upstream, ×2, ×3 of the upstream ranges),
   grouped (`optionGroups`). A per-column editor for the 14 shares is left
   out: one value off and the module silently ignores them all.

## Consequences

- Settings kept in tables are reachable from the UI with no new UI code
  (integer and choice options render as before).
- Each start runs a few small `UPDATE`s (milliseconds).
- Console changes to these columns last only until the next start; the
  option help says where the value lives.
- Module upgrades must re-check the table and columns (`bump-pins`).
