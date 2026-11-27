# Roadmap

Phases 0 to 5 are done; the launcher is public since 0.3.0 (2026-11-18).
Phase 6 goes on release by release, phase 7 proves a second emulator.

## Phase 0: foundations ✅

- [x] Repository layout, Rust workspace, Tauri 2 + Svelte 5 skeleton, FR/EN i18n
- [x] Manifest schema v1 (JSON Schema) and first manifests: AzerothCore,
      `playerbot` flavor, Playerbots / AHBot / Individual Progression, client 3.3.5a, MySQL 8.4
- [x] `rk-core` (model + resolver), `rk-catalog` (loader + validation), `rk` CLI
- [x] License (GPL-3.0-or-later), CONTRIBUTING, SECURITY
- [x] Git hooks (privacy, Conventional Commits, no AI co-author) + Claude Code hooks and skills
- [x] Git repository + public GitHub remote
- [x] CI: `cargo fmt/clippy/test`, `npm run check`, catalog validation (Windows) + portable crates (Linux)

## Phase 1: CI build chain ✅

- [x] Windows build of the Playerbot fork with all catalog modules (`build-server.yml`)
- [x] Verify `Updates.AllowedModules` naming (module directory name)
- [x] Prove disabled modules are inert (smoke test round A; AHBot needs its tables, ADR 0008)
- [x] MySQL 8.4 database snapshot made and validated by the smoke test (ADR 0009)
- [x] Signed index (ADR 0010): `tools/ci/index.mjs`, publish job (release per build + `index` release)
- [x] Signing key in the repository secrets; first publication (index serial 1, 2026-11-14)
- zstd packaging: moved to phase 5
- [x] Server data: pinned upstream release (ADR 0006)
- [x] Smoke tests: start with modules off/on (`tools/ci/smoke-test.mjs`, green 2026-11-14)
- [x] Smoke tests: authserver logon (SRP6, realm list), random bots online, AHBot
  auctions (ADR 0022)
- [x] AHBot account/character creation without a client (phase 3 post-install steps)
- **Exit:** a downloadable artifact that works when installed by hand ✅ (2026-11-14)

## Phase 2: install engine ✅

- [x] `rk-store`: parallel resumable downloads, mirrors, sha256 verification,
      per-blob lock, fail-fast on permanent errors
- [x] `rk-conf`: lossless `.conf` parser, layered rendering with provenance,
      hand-edit import
- [x] `rk-db` (MySQL portable: secure init, clean start/stop, users, backup/restore)
- [x] `rk-db` + engine: CI database snapshot restore (`db-claim`, `extract-db`, `db-adopt`,
      `rk install --db-snapshot`)
- [x] Job Object for stateless servers (`rk-runtime`, ADR 0011), synchronised folders
  refused (ADR 0020)
- [x] `rk-engine` executor: journal, resume, rollback, finalize, atomic directory swap
- [x] Built-in steps `download` + `extract`; `rk component install` (MySQL)
- [x] Steps `db-init/start/provision/stop`, `render-config`, `run-tool`; instance
      config in the manifest (ADR 0007); `rk install` (verified locally up to dbimport)
- [x] `rk install` end to end with the CI artifact; servers start (2026-11-14)
- [x] Signed remote index: verification, anti-rollback, selection by pins,
      `rk install --index` (verified locally end to end, 28 s)
- **Exit:** `rk install` creates a working instance from scratch

## Phase 3: runtime + AzerothCore driver ✅

- [x] Supervisor, readiness, graceful stop, logs (`rk-runtime`, ADR 0011; `rk start`,
  verified 2026-11-15: world ready in 21-92 s, clean stop, launcher kill -> world saves,
  auth dies, MySQL adopted at next start)
- [x] Driver: accounts (`rk account create`, SRP6 matches worldserver), realm address,
  console (stdin of worldserver)
- [x] Driver: AHBot account + character, module post-install steps (bindings, 2026-11-15:
  249 auctions per house on the first cycle)
- [x] Module lifecycle: enable / disable on an existing instance (`rk module`)
- [x] Module removal that needs the `world` database regenerated (Individual Progression:
  2 min 05 s, rollback restores the previous world)
- [x] Module update (new pinned commit) on an existing instance: `rk update` installs the
  build of the current catalog release, every database dumped first, rollback restores it
- [x] Log in and play with the maintainer's 3.3.5a client (2026-11-15: character created, in game, bots present)
- **Exit:** start, log in and play through the CLI

## Phase 4: client ✅

- [x] Detection: version resource of the executable (build 12340), locale folders (`rk client set`)
- [x] Realmlist (`realmlist.wtf` + the address saved in `Config.wtf`), cache cleared on server change
- [x] Module options stored per instance (`rk option set`), assets downloaded from the module's
  pinned commit (ADR 0013), server data overlay per instance
- [x] Managed client patches: tracked, player's files backed up and restored
- [x] Synchronised launch: `rk play` (servers stop when the game is closed)
- **Exit:** one command from a stopped instance to the login screen

## Phase 5: public app ✅ (except signing)

- [x] Shared services for the app and the CLI: `rk-launcher` (ADR 0014)
- [x] UI: install wizard (client folder, modules, solo/LAN), Play / start / stop with real
  server states, consoles (logs + commands), module toggles and options, accounts, resume,
  update, Repair
- [x] Per-user NSIS installer (no MSI: admin rights), `release-app.yml` (ADR 0015)
- [x] Tauri updater: signed feed `latest.json` in the `app-latest` release
- [x] Opt-in crash reports (local, redacted, GitHub issue submitted by the player)
- [x] zstd artifacts (measured 2026-11-15: server 256 → 173 MB, snapshot 144 → 94 MB;
  install from them 30 s): `tools/ci/zip-zstd.mjs` in the publish job
- [x] Updater key pair in the repository settings (2026-11-15)
- [x] First `release-app.yml` run: draft `app-v0.1.0` (unsigned, update signature made)
- [ ] Maintainer: SignPath Foundation project
- No beta phase: releases go public directly (decided 2026-11-18)

## Phase 6: player tools (ongoing since 2026-11-15)

- [x] Bots tab: Playerbots options (counts, levels, LFG/BG, guilds, chat), keys verified at
  the pinned commit
- [x] Backups tab + `rk backup`: manual player-data backups, retention of automatic dumps,
  restore of the player databases (ADR 0016; verified on `pubtest`: 11 s backup, 38 s restore)
- [x] Join a LAN realm from another computer (`rk join`, Home panel)
- [x] Realm settings: rates, realm type, factions, characters (ADR 0017; `GameType` copied to
  `realmlist.icon`)
- [x] Uninstall a realm (`rk uninstall`, last backup kept, shared downloads removed with the
  last realm) and choose the install location before the first install (ADR 0017)
- [x] Play while the servers run; terminals as a drawer over the page; console output decoded
  from the OEM code page
- [x] Beta readiness (ADR 0018): free-space checks and estimate, named port owners,
  diagnostic report, download speed and time left, window state and small screens
- [x] Player comfort (ADR 0018): game folder found automatically, live players / bots /
  uptime, notification area icon and hiding while playing, daily backups, release notes
  after a launcher update
- [x] Install location used as picked; renamed or moved data roots found again and their
  realms' paths updated (ADR 0019)
- [x] Data safety (ADR 0020): clean stop when Windows shuts down or logs off, kept awake
  while working, synchronised and non-NTFS folders refused, quarantined programs detected
- [x] GM panel (ready-made console commands, characters in the world, GM levels), start
  progress bar and "realm ready" notification, MultiBot client add-on (ADR 0020)
- [x] Several realms per game version: name, duplicate, export to a file, import on
  another computer (ADR 0020; verified on `pubtest`: copy 32 s, export 30 s)
- [x] FAQ and troubleshooting (EN/FR) linked from every error,
  reduced motion, keyboard-friendly dialogs, UI smoke test in CI (ADR 0020)
- [x] Project named RealmKeeper 0.2.0: logo, wordmark, `rk` names, new index signing key (ADR 0021)
- [x] LAN hosting firewall rule (optional elevation, principle 6; ADR 0022)
- [x] Backups while the servers run (players saved first); unused `assets/` removed (ADR 0022)
- [x] RealmKeeper 0.3.0: first public launcher release (2026-11-18, unsigned)
- [x] RealmKeeper 0.3.1: Play once online, account tools without sub-tabs, working GM
  panel, account work no longer races a start or a stop (2026-11-18)
- [x] RealmKeeper 0.3.2: six optional modules (transmog, autobalance, dungeon clear,
  enchanter, assistant, reagent bank) with their NPCs placed, safe realm import, bilingual release notes, title bar update
  button (2026-11-19)
- [x] RealmKeeper 0.3.3: a party of bots formed from the launcher (mod-realmkeeper), restore of a
  backup made by another server, game language choice, launcher options window, server
  update status (2026-11-21)
- [x] RealmKeeper 0.3.4: handoff between computers through a synchronised folder (ADR 0028),
  module settings kept in tables with AHBot quantities, quality mix and prices (ADR 0029),
  optional module SQL (ADR 0027), many more grouped module options, game cache cleared on
  content changes (2026-11-21)
- [x] RealmKeeper 0.3.5: German and Spanish, Server and Modules tabs with a new title bar,
  raid reset / profession / arena point settings, handoff sent when the launcher quits
  (2026-11-23)
- [x] RealmKeeper 0.3.6: flight path speed, instant flights and primary profession settings,
  No Hearthstone cooldown module, Playerbots account types of restored player accounts
  cleaned before each start (alts summoned with `.playerbots bot add` were re-levelled)
  (2026-11-24)
- [x] RealmKeeper 0.3.7: realms start again (the 0.3.6 account type cleanup failed on
  MySQL 8.4 with ERROR 1046) (2026-11-25)
- [x] RealmKeeper 0.3.8: database started alone for SQL tools (servers take it over),
  auction houses emptied from the AHBot options after a backup (ADR 0030) (2026-11-27)

## Phase 7: second emulator prototype (started 2026-11-19, ADR 0024)

Mists of Pandaria 5.4.8 with Playerbots (LoA-Pandaria-Playerbots), to prove
that an emulator is data + a driver.

- [x] Upstream facts verified; `v1.1.1` dumps import into MySQL 8.4.13 (94 s)
- [x] CI build of the pinned commit (servers + extractors): `build-server-pandaria.yml`,
  core pinned to release v1.1.1 (`2f9aebde`), green since 2026-11-19
- [ ] Publication of that build in the signed index
- [x] Catalog: client `wow-5-4-8`, emulator `pandaria` (marked `experimental`: listed only
  with `RK_EXPERIMENTAL=1`), flavor `playerbot`
- [x] Resumable game data extraction step from the player's client (`ExtractData`,
  `serverdata-<client>-<version>`)
- [x] `drivers/pandaria`: accounts, realm list, GM levels, population
- [ ] End-to-end install with a 5.4.8 client: extraction (mmaps: hours), login, bots online

## Later

- Authenticode signing of the installer (SignPath Foundation project, maintainer)
- Internet play (port forwarding help, account hardening)
- More modules and game versions in the catalog
- Optional "external database" mode, server auto-restart after a crash
- Linux / macOS (the core crates are kept portable and tested on Linux)

## Phase 8: Linux and the Steam Deck (started 2026-11-27, ADR 0031)

- [x] ADR 0031 (proposed): platform `linux-x86_64`, source key, Proton client
- [x] Deck checks: system libraries, MySQL minimal tarball, file systems, Proton (SteamOS 3.9.5)
- [ ] CI: Linux server build + Arch container smoke test, `sourceKey`, index entries (written, first run pending)
- [x] Engine: `.tar.xz` (links as hard links, Unix modes, also from release ZIPs), MySQL `extras` from Debian packages, executable suffix rule, other-platform builds and snapshots refused, source key in records, exports and handoff
- [ ] Runtime and launcher: process groups, `/proc` ports, logind inhibitor, data root
- [ ] Client through Proton (Steam shortcut, `proton run`, `wine`)
- [ ] AppImage + updater in `release-app.yml`
- [ ] Cross-platform restore in CI (Windows dump into the Linux smoke test)
- [ ] Handoff Windows <-> Deck verified (Syncthing folder)
- [ ] Later: action bars per computer, controller (ConsolePortLK + Steam Input)
