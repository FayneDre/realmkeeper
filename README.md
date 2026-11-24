# RealmKeeper

> Your own World of Warcraft server, installed in a few clicks.

[![CI](https://github.com/FayneDre/realmkeeper/actions/workflows/ci.yml/badge.svg)](https://github.com/FayneDre/realmkeeper/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/FayneDre/realmkeeper?filter=app-v*&label=launcher)](https://github.com/FayneDre/realmkeeper/releases/latest)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)

RealmKeeper is a free, open-source Windows launcher that installs, runs and
updates a private World of Warcraft server on your own computer, with its
modules, then starts **your own** game client on it. You never compile
anything, set up a database or edit a config file: the servers are built by
our CI and the launcher downloads them signed and verified.

The first supported game is **Wrath of the Lich King 3.3.5a** with
[AzerothCore](https://www.azerothcore.org) and
[Playerbots](https://github.com/mod-playerbots/mod-playerbots): a living world
filled with hundreds of bots that quest, group, trade and fight.

## Features

- **One-click realm**: download, install, start, log in. Your game folder is
  found automatically and pointed at the realm.
- **Modules on and off at any time**: auction house bot, Individual
  Progression (Vanilla → TBC → WotLK), transmogrification, AutoBalance,
  dungeon clear, enchanter, reagent banks and more. Their settings are
  grouped in plain forms, never raw config files.
- **Realm settings**: experience, drop, reputation and honor rates, realm
  type, cross-faction play, starting level and money, flight speed,
  professions, raid resets...
- **Bots**: how many, which levels, groups and battlegrounds, and a party of
  bots (role, class, spec, level) formed for the character you play.
- **Your data is safe**: daily backups, a full backup before every update,
  repair or restore, backups while the servers run, rollback when a change
  fails, clean stop when Windows shuts down.
- **Several realms**: duplicate a realm to try something, export it to a
  file, import it on another computer, or keep one realm in step between
  two computers through a folder you already synchronise.
- **Local network**: friends join your realm from their computer; the
  launcher can add the firewall rule for you.
- **GM panel**: accounts, GM levels, and ready-made commands (teleport,
  level, revive, send items or gold, announcements) without typing them.
- **Updates**: the launcher updates itself, and offers new server builds
  when the catalog moves to newer upstream versions.
- **No administrator rights, no Windows service**: everything lives in one
  folder you choose.
- In **English, French, German and Spanish**.

## Install

1. Download `RealmKeeper_<version>_x64-setup.exe` from the
   [latest release](https://github.com/FayneDre/realmkeeper/releases/latest)
   and run it. It installs for your Windows user only.
   The installer is not code-signed yet: if Windows SmartScreen says
   "Windows protected your PC", click **More info**, then **Run anyway**.
2. Open RealmKeeper, choose the game version, pick where the realm goes and
   which modules you want, then **Install**.
3. Create your account in the **Account** tab and click **Play**.

You need:

- Windows 10 or 11, 64-bit;
- about **10 GB** of free disk space per realm (the first install downloads
  about 1.8 GB);
- your own **World of Warcraft 3.3.5a** client (build 12340). RealmKeeper
  never downloads or shares the game client.

Questions and problems: see the [FAQ and troubleshooting](docs/faq.md)
([français](docs/faq.fr.md)); every error message in the launcher links to
the matching section.

## Supported games and modules

| Game | Server | Status |
|---|---|---|
| Wrath of the Lich King 3.3.5a | AzerothCore, [Playerbot fork](https://github.com/mod-playerbots/azerothcore-wotlk) | Available |
| Mists of Pandaria 5.4.8 | [LoA-Pandaria-Playerbots](https://github.com/1LineAtaTime/LoA-Pandaria-Playerbots) | Prototype, hidden ([ADR 0024](docs/decisions/0024-mists-of-pandaria-prototype.md)) |

Modules of the 3.3.5a realm (every one is compiled into the server build and
turned on or off from the launcher):

| Module | | Author |
|---|---|---|
| [Playerbots](https://github.com/mod-playerbots/mod-playerbots) | always on | mod-playerbots |
| [RealmKeeper party](https://github.com/FayneDre/mod-realmkeeper) | always on (bot party) | RealmKeeper |
| [Auction House Bot](https://github.com/azerothcore/mod-ah-bot) | optional | AzerothCore |
| [Individual Progression](https://github.com/ZhengPeiRu21/mod-individual-progression) | optional | ZhengPeiRu21 |
| [Transmogrification](https://github.com/azerothcore/mod-transmog) | optional | AzerothCore |
| [Auto Balance](https://github.com/azerothcore/mod-autobalance) | optional | AzerothCore |
| [Dungeon Clear](https://github.com/jrad7/mod-dungeon-clear) | optional | jrad7 |
| [Enchanter NPC](https://github.com/azerothcore/mod-npc-enchanter) | optional | AzerothCore |
| [Assistant](https://github.com/noisiver/mod-assistant) | optional | noisiver |
| [Reagent Bank](https://github.com/ZhengPeiRu21/mod-reagent-bank) | optional | ZhengPeiRu21 |
| [Reagent Bank (add-on)](https://github.com/Day36512/mod-reagent-bank-account) | optional | Day36512 |
| [No Hearthstone cooldown](https://github.com/BytesGalore/mod-no-hearthstone-cooldown) | optional | BytesGalore |

The exact upstream commits are pinned in [`catalog/`](catalog/). Want another
module or game? Open an issue: adding one is mostly data (a manifest) and,
for a new emulator, a small driver.

## How it works

- **No compilation on your computer.** GitHub Actions builds one server per
  catalog release with every module compiled in, starts it with bots in a
  smoke test, and publishes it with a database snapshot. A signed index
  (ed25519) tells the launcher which build matches its catalog; every
  download is checked by size and sha256.
- **Portable MySQL 8.4**, one per realm, listening on `127.0.0.1` only.
- **Transactional changes.** Installs, updates, module changes and restores
  are journaled steps: an interrupted change resumes, a failed one rolls
  back, and player databases are backed up before any migration.
- **Generic engine.** Emulators, clients and modules are described by JSON
  manifests; emulator-specific code lives in small drivers.

The details are in [`docs/architecture.md`](docs/architecture.md) and the
[architecture decisions](docs/decisions/); progress in
[`docs/roadmap.md`](docs/roadmap.md).

## Development

Windows, with [Rust](https://rustup.rs) (MSVC toolchain), Visual Studio 2022
Build Tools ("Desktop development with C++") and Node.js 24.

```sh
npm install                    # JS dependencies + repository git hooks
npm run dev                    # the launcher in development mode
npm run check                  # UI types, unit tests, catalog schema, i18n parity
cargo test --workspace         # Rust tests, including the real catalog
cargo run -p rk-cli -- --help  # `rk`, the command-line launcher
```

| Path | Content |
|---|---|
| `app/` | Tauri 2 + Svelte 5 app (thin adapter over the Rust crates) |
| `crates/` | Rust core: catalog, install engine, store, MySQL, runtime, client, launcher services, CLI |
| `drivers/` | Emulator drivers (AzerothCore, LoA Pandaria) |
| `catalog/` | Manifests of clients, components, emulators, flavors and modules |
| `tools/ci/`, `.github/workflows/` | Server build, smoke tests, signed index, launcher release |
| `docs/` | Architecture, roadmap, decisions, FAQ |

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) before your first pull request.

## License and legal

RealmKeeper is free software under the
[GNU General Public License v3.0 or later](LICENSE). Security issues: see
[`SECURITY.md`](SECURITY.md).

World of Warcraft is a trademark of Blizzard Entertainment. RealmKeeper is
not affiliated with or endorsed by Blizzard. The game client and Blizzard
assets are never distributed: you must provide your own client. Server
emulators and modules keep their own licenses (AzerothCore: GPL-2.0) and are
built from the exact source commits recorded in [`catalog/`](catalog/);
server data comes from its upstream release, never re-hosted.

Thanks to the AzerothCore and mod-playerbots communities and to every module
author listed above.
