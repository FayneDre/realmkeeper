# Security policy

RealmKeeper downloads and runs server software on players' machines, so
we take integrity seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private
[security advisory form](https://github.com/FayneDre/realmkeeper/security/advisories/new).
We aim to acknowledge reports within 7 days.

## Scope

- The launcher (`app/`, `crates/`), the catalog and its signing, CI release workflows.
- Out of scope: vulnerabilities in upstream emulators or modules (report them
  upstream; we will update pins once fixed).

## Design commitments

- **Verified downloads.** Server builds are listed in an ed25519-signed index
  (trusted keys compiled into the launcher, serial anti-rollback); every
  download is checked by size and sha256 before use, and archive entries with
  absolute paths, `..` or links are refused.
- **Built by CI only.** Server builds and launcher releases come from GitHub
  Actions workflows of this repository; actions are pinned by commit SHA.
  The launcher's auto-update feed is signed (Tauri updater key). Authenticode
  signing of the installer (SignPath Foundation) is not active yet.
- **Local by default.** MySQL listens on `127.0.0.1` only; SOAP stays off (the
  upstream default). In solo mode the game servers listen on `127.0.0.1`; in
  LAN mode they listen on the network, and the optional firewall rule is
  limited to this realm's programs on private and domain networks.
- **Credentials.** Database passwords are random and generated at install.
  They are stored in plain text inside the data root (`secrets.json`,
  `client.cnf`, the servers' `.conf` files, which upstream servers read in
  clear text), protected by the user's folder permissions; they are never
  passed on a command line.
- **No admin rights, no service, no telemetry.** Crash reports stay on the
  computer (redacted) until the player chooses to submit them.
- **Player files.** Realm exports and the files of the handoff between
  computers hold account names with their SRP6 password verifiers (not the
  passwords) and are not encrypted: keep them in personal folders.
