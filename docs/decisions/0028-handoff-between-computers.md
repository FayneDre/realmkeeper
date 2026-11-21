# 0028. Handoff between computers through a synchronised folder

- Status: Accepted
- Date: 2026-11-21

## Context

A player wants to play the same realm on two computers (at home and
elsewhere) and pick up where they stopped on the other one. Today this works
by hand: an export (`.rkrealm`, ADR 0020) the first time, then a manual
backup copied on a USB stick and restored on the other computer (ADR 0016).
It is easy to forget, and restoring an older state silently replaces what
was played since.

Facts and constraints:

- The player data of a realm is small next to the realm: one `.rkrealm` of
  the maintainer's realm (accounts, characters, ~1 300 Playerbots random
  bots) is ~25 MB (110 MB of SQL, deflate). Bots are characters of the
  world (guilds, auctions, groups) and travel with it.
- Synchronised folders (Google Drive for desktop `G:\My Drive`, OneDrive,
  Dropbox, Syncthing...) already move files between a player's computers,
  without an account or a server of ours. They are eventually consistent:
  a file may arrive late, partially, or not at all if the computer is shut
  down early, and two computers writing the same file name make "conflict
  copies".
- A data root inside a synchronised folder is refused (ADR 0020,
  `location.synced`): MySQL files must never be synchronised. Only dump
  files may travel.
- No server of ours, no admin rights, player data stays with the player
  (principles 6 and 8; scope v1 has no Internet play).
- Restore (ADR 0016/0026) already puts player databases back from a dump on
  any build: full dump first (`before-restore-*`), resumable, then dbimport
  migrates the data to the realm's build. It refuses a truncated dump.

Alternatives rejected: cloud APIs (OneDrive Graph, Google Drive API: app
registration, OAuth tokens, Google verification, for the same result as a
folder), a private GitHub repository (account and token: too technical),
a server of ours (cost, hosting personal data), playing remotely on the
home realm over a VPN (home computer must stay on, Internet play is out of
scope, often blocked at work).

## Decision

1. **Handoff folder.** One folder per computer, chosen in the "Other
   computer" window (`folder` in `<app dir>/handoff.json`). The launcher
   suggests `<Google Drive>\RealmKeeper` or `<OneDrive>\RealmKeeper` when it
   detects one (`location::synced_folders`), any folder is accepted
   (network share, Syncthing). It must exist, be writable, and neither hold
   nor sit inside the data root. The launcher never talks to a cloud
   service: the sync client moves the files.

2. **Identity.** Each launcher has a random `machineId` (16 hex,
   `<app dir>/handoff.json`) and shows the computer name (`COMPUTERNAME`) as its
   label. A shared realm has a random `uid` (32 hex). The link of a local
   realm to the folder is `instances/<id>/handoff.json`:
   `{format, uid, generation, played, pulling?}`. It is not part of the
   instance record, not copied by a realm duplicate and not carried by an
   export: a copy or an import is a new, unshared realm.

3. **Folder layout**, one directory per shared realm, every file written
   by one computer only (no two computers ever write the same name, so no
   conflict copies):

   ```
   <folder>/<uid>/
     state-<generation>-<machineId>.rkrealm   an export (ADR 0020): realm.json, bindings.json, player data
     head-<machineId>.json                    written last: the newest state this computer wrote
     playing-<machineId>.json                 servers running here since <secs>; removed at a stop
   ```

   `head-*.json`: `{format, uid, generation, parent, machineId, machine,
   secs, emulator, file, size, sha256, build, launcher, name}`. State and head are
   written to `*.tmp` and renamed; the head comes after the state. A head
   whose state file is missing or has another size or sha256 is **not yet
   arrived** and never used.

4. **Generations.** A state's `generation` is one more than the newest one
   known when it was written; `parent` is the generation the writer started
   from. Locally, `generation` is the last one this computer wrote or took
   back, `played` says the servers ran here since.

5. **Decision** (a pure function of `handoff.json` and the verified heads,
   unit-tested on every case). `newest` = the head of highest generation
   (ties: newest `secs`, then `machineId`).

   | Folder | Played here | Result |
   |---|---|---|
   | unreachable | any | `unavailable`: play normally, send later |
   | no head, or `newest.generation <= generation` | no | `upToDate` |
   | no head, or `newest.generation <= generation` | yes | `toSend` |
   | `newest.generation > generation`, arrived | no | `toTake` (offer to take it back) |
   | `newest.generation > generation`, not arrived | no | `arriving` (say so, play allowed) |
   | `newest.generation > generation` | yes | `conflict` |

   An other computer's `playing-*.json` younger than 12 h adds a warning
   ("still playing on PC-X?") without blocking: that computer may have
   been switched off without a clean stop.

6. **Send at every clean stop.** When a session starts the servers,
   `played = true` and `playing-<machineId>.json` is written. After a clean
   stop (like the daily backup, ADR 0018, in the background and in the
   realm's turn) the launcher exports the realm to the folder as generation
   `max(generation, newest) + 1` with `parent = generation`, writes the
   head, removes the playing file, then sets `generation` and
   `played = false`. A stop that was not clean (crash, power loss) leaves
   `played` set: the next clean stop, or "Send now", sends. A send is
   refused when the result is `conflict` or `toTake` (it would hide the
   other computer's play). After a send, this computer's own state files
   older than the two newest generations of the folder are deleted.

7. **Take back.** "Take back the game of PC-X (date)" replaces Play in the
   hero while the result is `toTake`. It verifies the state file (size,
   sha256), copies its dump into the realm's backups as
   `handoff-<generation>-<secs>.sql` (kind "From another computer", listed
   and restorable), applies the other computer's choices when they differ
   (modules, module options, realm settings: one module operation, ADR
   0012 rebuild rules), then runs the restore (full dump first,
   resumable). `pulling` in `handoff.json` names the generation being
   taken; `generation` and `played = false` are set when the restore ends,
   also when it ends through `resume`. Refused when the state comes from a
   newer launcher than this one (`handoff.launcherOlder`) or when the
   realm has a server update waiting (`handoff.updateFirst`, update then
   take back): player data never goes to an older schema.

8. **Conflict.** Both computers played since the last handoff. The
   launcher never merges nor chooses: it shows both (computer, date) and
   the player picks. "Keep this computer's game" sends with a generation
   above the other one; "Take the other computer's game" takes it back
   (this computer's state is in `before-restore-*`). The losing state is
   never deleted by the choice: the other computer's file stays in the
   folder until that computer prunes it, this one's in the backups.

9. **Sharing a realm and joining it.** "Share" on a stopped realm creates
   the `uid`, `handoff.json` and sends generation 1. On another computer
   with the same folder, realms of the folder whose `uid` no local realm
   has are listed with their newest head ("Garona, shared from PC-HOME");
   "Install" imports the newest arrived state (ADR 0020 import, resumable)
   and writes `handoff.json` with that `uid` and generation. "Stop
   sharing" removes `handoff.json` (the folder keeps its files; deleting
   them is a separate, confirmed action).

10. **Security.** The state holds account names, SRP6 salts and verifiers
    (never passwords) and the characters. It is not encrypted in this
    version: it stays in the player's own cloud. The option help says to
    use a personal synchronised folder, never one owned by an employer
    (e.g. "OneDrive - Company"). Encryption with a passphrase may come
    later as a new state format.

11. **Surfaces.** `rk-launcher::handoff` (folder checks, heads, decision,
    send, take back, share, join, prune), used by the app and by
    `rk handoff folder [<path>] | status <id> | share <id> | unshare <id> |
    send <id> [--keep-mine] | take <id> | list | join <uid> <id>` (the
    computer's settings in `--root`, or `RK_HANDOFF_DIR` to play two
    computers on one in tests; `rk start` / `rk play` mark and send like
    the app). UI: an "Other computer" button in the realm bar opens a
    window (folder, this realm shared or not, realms of the folder to
    install); the realm's handoff state is its status in the hero ("Newer
    game elsewhere" / "Two different games", action in place of Play), a
    conflict modal, a toast after a send; the app refuses to start a realm
    whose state is `toTake` or `conflict`. Errors `handoff.*`, FAQ entry
    `#handoff`.

## Consequences

- Two computers can share a realm with nothing but a folder the player
  already synchronises; the first install on the second computer needs no
  USB stick either.
- Each clean stop costs one export (~30 s for the maintainer's realm, in
  the background) and ~25 MB of upload; two states per computer stay in
  the folder.
- The handoff is only as fresh as the sync client: switching off right
  after a stop may leave the state unsent; the other computer then sees
  `arriving` or nothing new, and a later play there ends in `conflict`
  rather than in lost play.
- Restores take minutes (dump, replay, dbimport): taking back is not
  instant. A differential format could come later.
- Unencrypted player data in a cloud folder is the player's choice; the
  help says so.
- Not covered: playing the same realm at the same time on two computers
  (that is LAN or Internet play), more than one folder per computer.
