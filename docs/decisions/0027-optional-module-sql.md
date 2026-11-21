# 0027. Optional module SQL files as options

- Status: Accepted
- Date: 2026-11-21

## Context

Some modules ship SQL files that are not applied by default and that the
server owner is meant to choose. Individual Progression has 22 of them in
`optional/sql/world/zz_optional_*.sql` at the pinned commit (`e179555`):
phasing of content added by later patches, class spells taught only in
their own expansion, heirlooms removed, Vanilla stack sizes, potion and
crafting cooldowns, creature models, regeneration values, landmines in
Alterac Valley, but also changes that make the game easier (lighter
Ahn'Qiraj quests, TBC heroic keys at Honored, weaker TBC raid bosses).
Upstream asks the owner to run the chosen files against the world database
by hand.

Facts of the Playerbot fork's dbimport (`UpdateFetcher.cpp` @ 32fd347):

- For every module named in `Updates.AllowedModules`, it reads every
  directory under `modules/<dir>/data/sql/` whose name contains the
  database's module name (`world`, `characters`, `auth`), recursively.
- All files are applied in the order of their **file names** (the
  `zz_optional_` prefix puts these last) and recorded in the `updates`
  table; two files with the same name abort it.
- Applied files of state `MODULE` are never cleaned up when they disappear
  from disk: removing a file does not undo it.

Four of the 22 files need patch-V or patch-S on the client and the server
(rogue poisons, spell damage and healing, TBC PvP prices, Vanilla
transports, per their own headers).

## Decision

1. **A new option type `sql`** in module manifests: `database` (a catalog
   database the module touches, never one with player data), `file` (path
   in the module repository, `.sql`), boolean `default`. Its value is a
   boolean in `instance.json`, like a `boolean` option; the UI shows a
   checkbox. Realm settings cannot be `sql`.
2. **Packaged by CI.** `build-plan.mjs` lists each module's `sql` files in
   the plan (`optionalSql`, part of the build key) and `package-server.mjs`
   copies them from the pinned module sources into the artifact, at
   `sql/modules/<id>/<file>`; a missing file fails the build. No new
   download path, no per-file hash in the catalog: the artifact's
   `files.json` covers them.
3. **Where they go: declared by the emulator.** `instance.optionalSql` of
   the emulator manifest gives the `source` in the SQL tree
   (`modules/{module}/{file}`) and the `target` directory dbimport reads
   (`modules/{module}/data/sql/db-{database}/rk-optional`). The engine knows
   no emulator layout.
4. **Step `optional-sql`**, before `render-config` and `db-import` in the
   module operation and in the update operation (a new SQL tree has none of
   the files): every module with `sql` options gets its target directories
   holding exactly the files turned on (none when the module is disabled).
   Idempotent, nothing to undo. A file missing from an older server build
   fails with "update the realm".
5. **Turning a file off rebuilds its database**, like removing a module
   (ADR 0012): `set_options` adds the database to `regenerate` when an `sql`
   option goes from on to off. Turning one on only adds the file; dbimport
   applies it on top. If the operation turning a file on fails, the record
   goes back to the previous options **plus** a rebuild of that database
   (`after_failure`): dbimport may have run part of the file.
6. **Individual Progression** offers the 18 files that need no client
   patch, all off by default (upstream: optional). Labels say which ones
   make the game easier. The four files that need patch-V/S wait for a
   dependency between options.
7. **Smoke test.** Round B puts every optional file of the enabled modules
   in place before dbimport, so a file that no longer applies on a pinned
   build fails CI.

## Consequences

- Each file is a checkbox: Individual Progression's options list grows to 22
  lines.
- Turning a world file off costs a world rebuild (about 2 minutes); player
  databases are never touched, as for module removal.
- Realms installed from an older server build must update before turning a
  file on (clear error, no silent skip).
- A realm duplicated with hard links (ADR 0020) shares its SQL files: the
  step writes through a new file (atomic rename) and removes only its own
  links, so the other realm keeps its files.
- `bump-pins` must check that every `sql` option file still exists at the
  new commit (CI checks it too, when packaging).
- A pin bump that drops or renames a file a realm has turned on leaves its
  changes in the world until the next rebuild: `bump-pins` checks the files.
- Options that depend on another option (the four patch-V/S files) need a
  later decision.
