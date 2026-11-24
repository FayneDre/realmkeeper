# Catalog

Declarative description of everything the launcher can install. The launcher
engine is generic; adding an emulator, client, component or module starts here.

```
catalog/
├─ schema/                         JSON Schemas (draft 2020-12), one per manifest kind
├─ clients/<id>.json               game clients (detection, realmlist, locales, server data, branding)
├─ components/<id>.json            shared runtimes (MySQL 8.4)
└─ emulators/<emulator>/
   ├─ emulator.json                driver, client, databases, ports, processes, instance config,
   │                               realm settings, GM panel actions, bot party
   ├─ flavors/<id>.json            core source trees (official repo or fork), required modules
   └─ modules/<id>.json            modules (activation, SQL, config, options, assets, add-ons,
                                   NPC spawns, post-install steps)
```

Today: `wow-3-3-5a` + `azerothcore` (`playerbot` flavor, 12 modules) and the
hidden Mists of Pandaria prototype (`wow-5-4-8` + `pandaria`, marked
`experimental`, listed only with `RK_EXPERIMENTAL=1`).

## Rules

- File name == `id` (kebab-case). Flavors/modules: `emulator` == parent directory.
- A module `id` is also its directory in the server build and its `dbimportId`:
  it must be the name the module's loader expects (`Add<id with - → _>Scripts()`).
- Every upstream source is pinned to a full 40-char commit; `ref` only documents the branch.
- User-facing text is `{ "en": "...", "fr": "...", "de": "...", "es": "..." }`;
  English is mandatory and the fallback.
- Config values follow upstream `.conf` conventions (booleans are `0`/`1`).
- A module listed in a flavor's `requiredModules` must be `"removable": false`.
- A removable module must use `activation.mode = "config"` and be inert when
  disabled; if it reads its own tables even while disabled, set
  `database.applyWhenDisabled` (ADR 0008).
- Option defaults are the upstream defaults; never throttle in-game behaviour.
- Assets (client patches, add-ons, DBC overrides) are archives of the module
  repository at its pinned commit, with `size` and `sha256` (ADR 0013): never
  re-hosted, never committed.
- Schema changes must be mirrored in `crates/rk-core/src/manifest.rs` (and vice versa).
- Adding or bumping a module changes the build key: the next `build-server`
  run compiles a new server. Re-sign the test index fixtures
  (`crates/rk-catalog/tests/fixtures/`) with the new pins.

The `add-module`, `add-emulator` and `bump-pins` skills in `.claude/skills/`
describe the full procedures, including the upstream facts to verify.

## Validate

```sh
npm run validate:catalog                      # JSON Schema + layout
cargo run -p rk-cli -- catalog validate      # cross-manifest references
cargo test -p rk-catalog                     # tests against the real catalog
cargo run -p rk-cli -- catalog resolve --emulator azerothcore --module mod-ah-bot
```
