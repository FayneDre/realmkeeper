# 0031. Linux and the Steam Deck

- Status: Proposed
- Date: 2026-11-27

## Context

The maintainer plays on a Windows PC and on a Steam Deck (SteamOS 3, an
immutable Arch Linux: read-only root, no system packages added by the
player). They have run AzerothCore with 2 500 Playerbots and the 3.3.5a
client on the Deck itself: the whole realm must run there, not only a client
joining the PC. The same realm must keep moving between both computers
(handoff, ADR 0028). Principles 1 (no compilation on the player's machine)
and 2 (never throttle the game) still hold.

What the code already has: the signed index selects builds by `platform`
(`windows-x86_64` today, `rk-catalog` `index.rs`), components declare
binaries per platform (`mysql-8-4` `platforms`), the portable crates are
tested on Linux in CI, and player data travels as SQL dumps (exports,
handoff, backups). What is Windows-only: the emulator manifest names
`bin/*.exe`; the supervisor uses Job Objects; port owners come from
`netstat`/`tasklist`; the firewall rule; shutdown blocking and power
requests; directory junctions for the data overlay; the OEM code page of the
console; the NSIS installer; `Wow.exe` launched directly.

Upstream facts (2026-11-27):

- The Playerbot fork builds on Linux in its own CI (`core-build-playerbots.yml`
  @ 32fd347: Ubuntu 22.04/24.04, gcc/clang, system Boost, OpenSSL,
  libmysqlclient, readline, ncurses, bz2). `deps/boost/CMakeLists.txt` asks
  for static Boost on Windows only; on Linux the servers load the OpenSSL 3
  `legacy` provider from OpenSSL's own module directory
  (`OpenSSLCrypto.cpp`: `SetupLibrariesForWindows` is Windows-only). On
  Linux the config directory is the compile-time `_CONF_DIR`, from the CMake
  `CONF_DIR` (default `<prefix>/etc`), not `configs/` as on Windows
  (`Config.cpp` `GetConfigPath`, `src/cmake/showoptions.cmake`).
- MySQL 8.4.13 `linux-glibc2.28-x86_64-minimal.tar.xz`: 79 753 532 B,
  sha256 `5dfbb764f731df40ef9e4f5d69aed32bdac8ba1af20bd96b7d40a942d83d4ed2`
  (MD5 `fb536f5d26e3e43e933aec34679c0740`; not yet cross-checked with
  dev.mysql.com, which refuses scripts), on `cdn.mysql.com/Downloads/MySQL-8.4/`.
  It holds `mysqld`, `mysql`, `mysqldump`, `mysqladmin` and
  `libmysqlclient.so.24`; the binaries find their own OpenSSL, protobuf and
  Abseil in `lib/private` (`RPATH $ORIGIN/../lib/private`), but `mysqld`
  also needs `libaio.so.1` and `libnuma.so.1`, and `mysql` needs
  `libncurses.so.6` / `libtinfo.so.6`, from the system.
- A MySQL data directory keeps the `lower_case_table_names` it was
  initialised with (1 by default on Windows, 0 on Linux) and refuses to start
  with another value: a database snapshot made on Windows (ADR 0009) cannot
  be used on Linux. SQL dumps are not affected (AzerothCore, module and
  Playerbots table names are lower case; to be confirmed by the cross-platform
  restore test below).

## Decision

1. **Platform `linux-x86_64`**, first target SteamOS 3 on the Steam Deck,
   also desktop distributions with glibc 2.35 or newer. Same features as
   Windows, except the LAN firewall rule (SteamOS has no active firewall;
   the panel is hidden) and the Mists of Pandaria prototype (Windows only
   for now).
2. **Server build in CI**, in `build-server.yml`, as a second build job on
   `ubuntu-22.04` (glibc 2.35, older than SteamOS) from the same plan: same
   catalog pins, so both platforms of a catalog release are published by the
   same run, and the index gets one entry per platform. Each platform has its
   own build key (its job's recipe). The artifact has the Windows layout:
   programs at the root of `dist/`, `configs/` with the `.conf.dist`, and
   `lib/`. Portability: Boost linked statically
   (`-DBoost_USE_STATIC_LIBS=ON`); `libmysqlclient.so.24` taken from the
   pinned MySQL component; every library outside the system base copied
   into `lib/` (`tools/ci/bundle-libs.sh`, `RUNPATH $ORIGIN/lib`, the copies
   `$ORIGIN`), and CI fails when a binary still resolves one elsewhere. The
   system base: glibc, libstdc++ / libgcc_s, zlib and OpenSSL 3 (on the Deck
   with its `legacy` provider in `/usr/lib/ossl-modules`; a bundled
   libcrypto would look for its providers at the build machine's path).
   `-DCONF_DIR=configs`: that value is also compiled into the servers
   (`_CONF_DIR`, `Config.cpp` `GetConfigPath` on Linux), so they read
   `configs/` and `configs/modules/` in their working directory, the
   instance root, as on Windows. The Linux smoke test runs in an
   `archlinux` container (the base of SteamOS), so a missing library fails
   in CI, not on the Deck.
3. **Source key.** Next to the build key, `build-info.json` and each index
   entry carry a platform-independent `sourceKey` (hash of the core and
   module plan entries). Player data moves freely between builds with the
   same source key: handoff, exports, restores and the "an older build lacks
   this" checks compare source keys, never build keys across platforms. A
   realm taken back from the PC on the Deck needs the Linux build of the same
   catalog release, which the same CI run published. The installed realm
   keeps its build's source key (`instance.json` `source`), exports and
   handoff heads carry it (`source`, ignored by older launchers), and a take
   back asks for the server update first when the offered build has the
   other computer's source key. Without a source key on one side the build
   keys are compared, as before.
4. **Database.** The `mysql-8-4` component gets a `linux-x86_64` entry: the
   minimal tarball above. `rk-engine` unpacks `.tar.xz` (pure Rust, one
   stream; its symbolic links become hard links to files of the same
   archive, so nothing unpacked points outside the component), then copies
   the declared `extras` out of their Debian packages (`package-files` step,
   inside the component operation: a failure rolls the whole component
   back). The database snapshot is made per platform by that platform's
   smoke test and carries the platform in `snapshot.json`; a snapshot or a
   server build of another platform is refused (`build-info.json` and
   `snapshot.json` without one are Windows ones). The Linux smoke test also restores a player-data dump made
   by the Windows smoke test (cross-platform check of point 3).
5. **Executables.** The emulator manifest names programs without extension
   (`bin/worldserver`); the engine adds the platform's executable suffix
   (`.exe` on Windows). Installed Windows realms keep working: the rule
   produces the same paths. Release ZIPs of Linux builds record Unix
   permissions (`zip-zstd.mjs --unix-modes`; GitHub artifacts lose them) and
   the engine applies them when it unpacks.
6. **Runtime and system.** In `rk-runtime` and `rk-launcher`, behind
   `cfg(target_os)`:
   - processes: a process group per server, killed for `terminate`, and
     `PR_SET_PDEATHSIG` so that `close-stdin` servers still see their stdin
     close when the launcher dies (same behaviour as the Job Object);
   - port owners from `/proc/net/tcp{,6}` and `/proc/<pid>` (no `ss`);
   - console output is UTF-8 (no OEM code page);
   - shutdown, log-off and sleep: a systemd-logind `delay` inhibitor
     (`shutdown:sleep`) held while servers or an operation run; the clean
     stop runs before it is released. Sleep is common on the Deck: a stop
     that cannot finish in the delay is left to InnoDB recovery, as after a
     crash;
   - data root: `$XDG_DATA_HOME/RealmKeeperData` (`~/.local/share`) by
     default; the location check accepts ext4, btrfs and xfs (the Deck's
     internal drive and microSD cards formatted by SteamOS are ext4) and
     refuses vfat/exFAT/NTFS (no hard links or symbolic links as needed);
     directory junctions become symbolic links.
7. **Client through Proton.** `Wow.exe` stays a Windows program. On Linux
   `rk-client` launches it, in order of preference: through Steam, as a
   non-Steam shortcut with a Proton compatibility tool (keeps Steam Input,
   the overlay and Game Mode; details in a later ADR with the controller
   work); else directly with the newest installed Proton (`proton run`,
   `STEAM_COMPAT_DATA_PATH` under the data root,
   `STEAM_COMPAT_CLIENT_INSTALL_PATH`); else `wine` when installed. "The
   game exited" is watched on the `Wow.exe` process, not on the child the
   launcher started. Client detection (version resource) is unchanged.
8. **App.** Tauri on WebKitGTK, packaged as an AppImage by `release-app.yml`
   (an Ubuntu 22.04 job next to the Windows one), with the Tauri updater
   (`latest.json` gains a `linux-x86_64` entry). Window and tray code in
   `app/src-tauri/src/os.rs` gets a Linux side or is skipped. Gamepad use of
   the launcher in Game Mode is a later step.
9. **Handoff on SteamOS.** ADR 0028 needs a folder the player already
   synchronises. Google Drive and OneDrive have no Linux client on SteamOS:
   the FAQ recommends Syncthing (on both computers) or an rclone sync; the
   launcher itself does not change.

Checks on the Deck before the parts that depend on them are written: SteamOS
and glibc versions; presence of `libaio.so.1`, `libnuma.so.1`,
`libncurses.so.6`, `libtinfo.so.6`, `libreadline.so.8`, `libbz2.so.1.0`,
`libssl.so.3` and `/usr/lib/ossl-modules`; the minimal MySQL tarball's
programs starting (`--version`); the file system of `/home` and of the
microSD card; installed Proton versions.

**System libraries MySQL lacks are ours, not the system's.** Installing
packages on SteamOS needs `steamos-readonly disable`, a `sudo` password many
players never set, and is wiped by every SteamOS update (principle 6: no
admin rights). The `linux-x86_64` entry of `mysql-8-4` therefore lists extra
files: the official Arch Linux packages of `libaio` and `numactl` (LGPL-2.1),
pinned by URL on `archive.archlinux.org` and sha256, downloaded from there
(never re-hosted, like ADR 0006), whose `libaio.so.1` / `libnuma.so.1` the
engine extracts into MySQL's `lib/private` (searched first: the binaries'
`$ORIGIN/../lib/private` path). Always, not only when the system lacks them:
the MySQL that runs is the one tested in the Arch container. Any other
library the Deck check finds missing gets the same treatment.

Deck check (SteamOS 3.9.5, build 20261023.100, glibc 2.43, 2026-11-27):
the minimal tarball's `mysqld`, `mysqldump` and `mysqladmin` start
(`--version`), so `libaio.so.1` and `libnuma.so.1` are present today (still
bundled, as above); `mysql` fails with `libncurses.so.6: cannot open shared
object file` (Arch ships only the wide `libncursesw.so.6`). The narrow
`libncurses.so.6` and `libtinfo.so.6` come from Debian 12's `libncurses6` /
`libtinfo6` 6.4-4 packages, by their permanent `snapshot.debian.org` URLs
(sha256 pinned): the engine unpacks a `.deb` (`ar` + `data.tar.xz`). The
OpenSSL legacy provider is at `/usr/lib/ossl-modules/legacy.so`. `/home`
is ext4 on the internal NVMe drive; the microSD card is ext4 (mounted under
`/run/media/deck/<label>`). Installed: Proton 7.0, 9.0 (beta), 10.0, 11.0,
Experimental, Hotfix, and GE-Proton / UMU-Proton in
`compatibilitytools.d`.

## Consequences

- One catalog release now means two server builds (about one more hour of
  CI per release) and two snapshots; the index doubles.
- Moving player data between Windows and Linux depends on the source key:
  a catalog release whose Linux build failed cannot be taken back on the
  Deck until it is fixed (the realm stays on its build, the handoff says so).
- `build-plan.mjs` and the workflow change, so the Windows build key changes
  once: every Windows realm is offered a rebuilt, identical server. Best
  shipped together with the next catalog change.
- More `cfg(target_os)` code in the runtime; CI already runs the portable
  crates on Linux and will run the full workspace there.
- Controller setup and per-device action bars build on this (later ADRs).
