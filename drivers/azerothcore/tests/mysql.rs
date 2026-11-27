//! Every statement the AzerothCore driver sends, run on a real MySQL 8.4.
//!
//! Skipped unless `RK_MYSQL_BASEDIR` points to an unpacked MySQL 8.4 ZIP
//! (the directory containing `bin/mysqld`); the Windows CI sets it.
//!
//! The unit tests only look at the SQL text: 0.3.6 shipped a multi-table
//! `DELETE` that MySQL rejects without a default database (ERROR 1046) and
//! no realm could start. Here the driver runs through the same
//! `rk_db::Instance::execute` as a real start, on the tables it touches,
//! with the columns and keys of the Playerbot fork @ `Playerbot` and of
//! mod-playerbots (subset: only the columns the driver reads or writes, and
//! those without a default).

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;

use rk_db::{Binaries, Instance, ServerSettings};
use rk_driver_azerothcore::AzerothCore;
use rk_engine::install::{InstanceRecord, Network};
use rk_runtime::{DriverContext, EmulatorDriver, NewAccount, NewCharacter, Population};

const SCHEMA: &str = "
CREATE TABLE acore_auth.account (
  id int unsigned NOT NULL AUTO_INCREMENT,
  username varchar(32) NOT NULL DEFAULT '',
  salt binary(32) NOT NULL,
  verifier binary(32) NOT NULL,
  email varchar(255) NOT NULL DEFAULT '',
  reg_mail varchar(255) NOT NULL DEFAULT '',
  joindate timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  expansion tinyint unsigned NOT NULL DEFAULT '2',
  PRIMARY KEY (id),
  UNIQUE KEY idx_username (username)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE acore_auth.account_access (
  id int unsigned NOT NULL,
  gmlevel tinyint unsigned NOT NULL,
  RealmID int NOT NULL DEFAULT '-1',
  comment varchar(255) DEFAULT '',
  PRIMARY KEY (id, RealmID)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE acore_auth.realmcharacters (
  realmid int unsigned NOT NULL DEFAULT '0',
  acctid int unsigned NOT NULL,
  numchars tinyint unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (realmid, acctid),
  KEY acctid (acctid)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE acore_auth.realmlist (
  id int unsigned NOT NULL AUTO_INCREMENT,
  name varchar(32) NOT NULL DEFAULT '',
  address varchar(255) NOT NULL DEFAULT '127.0.0.1',
  localAddress varchar(255) NOT NULL DEFAULT '127.0.0.1',
  port smallint unsigned NOT NULL DEFAULT '8085',
  icon tinyint unsigned NOT NULL DEFAULT '0',
  flag tinyint unsigned NOT NULL DEFAULT '2',
  gamebuild int unsigned NOT NULL DEFAULT '12340',
  PRIMARY KEY (id),
  UNIQUE KEY idx_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
INSERT INTO acore_auth.realmlist (id, name, address, localAddress, port, icon)
  VALUES (1, 'AzerothCore', '10.0.0.9', '10.0.0.9', 8085, 0);
CREATE TABLE acore_characters.characters (
  guid int unsigned NOT NULL DEFAULT '0',
  account int unsigned NOT NULL DEFAULT '0',
  name varchar(12) NOT NULL,
  race tinyint unsigned NOT NULL DEFAULT '0',
  class tinyint unsigned NOT NULL DEFAULT '0',
  gender tinyint unsigned NOT NULL DEFAULT '0',
  level tinyint unsigned NOT NULL DEFAULT '0',
  money int unsigned NOT NULL DEFAULT '0',
  position_x float NOT NULL DEFAULT '0',
  position_y float NOT NULL DEFAULT '0',
  position_z float NOT NULL DEFAULT '0',
  map smallint unsigned NOT NULL DEFAULT '0',
  orientation float NOT NULL DEFAULT '0',
  taximask text NOT NULL,
  online tinyint unsigned NOT NULL DEFAULT '0',
  zone smallint unsigned NOT NULL DEFAULT '0',
  at_login smallint unsigned NOT NULL DEFAULT '0',
  innTriggerId int unsigned NOT NULL,
  PRIMARY KEY (guid),
  KEY idx_account (account),
  KEY idx_online (online),
  KEY idx_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE acore_world.creature_template (
  entry int unsigned NOT NULL DEFAULT '0',
  name char(100) NOT NULL DEFAULT '0',
  PRIMARY KEY (entry)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE acore_world.creature (
  guid int unsigned NOT NULL AUTO_INCREMENT,
  id int unsigned NOT NULL DEFAULT '0',
  map smallint unsigned NOT NULL DEFAULT '0',
  position_x float NOT NULL DEFAULT '0',
  position_y float NOT NULL DEFAULT '0',
  position_z float NOT NULL DEFAULT '0',
  orientation float NOT NULL DEFAULT '0',
  spawntimesecs int unsigned NOT NULL DEFAULT '120',
  Comment text,
  PRIMARY KEY (guid)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE acore_world.game_tele (
  id int unsigned NOT NULL,
  position_x float NOT NULL DEFAULT '0',
  position_y float NOT NULL DEFAULT '0',
  position_z float NOT NULL DEFAULT '0',
  orientation float NOT NULL DEFAULT '0',
  map smallint unsigned NOT NULL DEFAULT '0',
  name varchar(100) NOT NULL DEFAULT '',
  PRIMARY KEY (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
INSERT INTO acore_world.game_tele VALUES
  (1, -8833.38, 628.628, 94.0066, 0, 0, 'Stormwind'),
  (2, 1629.85, -4373.64, 31.5573, 0, 1, 'Orgrimmar');
INSERT INTO acore_world.creature_template (entry, name) VALUES
  (190010, 'Warpweaver'), (601015, 'Beauregard Boneglitter'), (290011, 'Reagent Banker');
INSERT INTO acore_world.creature (guid, id, map) VALUES (5300678, 1, 0);
";

/// Created by worldserver at its first start, so absent before it.
const PLAYERBOTS_SCHEMA: &str = "
CREATE TABLE acore_playerbots.playerbots_account_type (
  account_id int unsigned NOT NULL,
  account_type tinyint unsigned NOT NULL DEFAULT 0,
  assignment_date timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (account_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
";

/// Modules whose NPCs the launcher places (ADR 0023).
const SPAWN_MODULES: [&str; 3] = ["mod-transmog", "mod-npc-enchanter", "mod-reagent-bank"];

/// A port of its own for each test server. Not one the system hands out
/// (`bind` on port 0): such a port is free when picked, but MySQL binds it
/// only after its initialisation, a minute later, and a test running in
/// parallel could get it meanwhile. These are below the dynamic ranges of
/// Windows (49152+) and Linux (32768+), and skipped when taken.
fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(26000);
    loop {
        let port = NEXT.fetch_add(1, Ordering::Relaxed);
        assert!(port < 26000 + 2000, "no free test port");
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

/// Stops the test server even when the test fails.
struct StopOnDrop<'a>(&'a Instance);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let instance = self.0.clone();
        let _ = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _ = runtime.block_on(instance.stop());
        })
        .join();
    }
}

async fn query(db: &Instance, sql: &str) -> Vec<String> {
    db.execute(sql)
        .await
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn account(username: &str, gm_level: u8) -> NewAccount {
    NewAccount {
        username: username.into(),
        password: "secret".into(),
        gm_level,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn driver_statements_run_on_mysql() {
    let Some(basedir) = std::env::var_os("RK_MYSQL_BASEDIR") else {
        eprintln!("RK_MYSQL_BASEDIR not set, skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Instance::new(
        dir.path().join("mysql"),
        Binaries::from_basedir(basedir),
        ServerSettings {
            port: free_port(),
            buffer_pool_mib: 128,
            ..Default::default()
        },
    );
    db.initialize().await.unwrap();
    db.start().await.unwrap();
    let _stop = StopOnDrop(&db);
    for name in [
        "acore_auth",
        "acore_characters",
        "acore_world",
        "acore_playerbots",
    ] {
        db.create_database(name).await.unwrap();
    }
    db.execute(SCHEMA).await.unwrap();

    // The real manifests: database names, spawns, Playerbots database.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    let catalog = rk_catalog::load_dir(&root).expect("catalog loads");
    let record = InstanceRecord {
        format: 1,
        id: "test".into(),
        emulator: "azerothcore".into(),
        flavor: "playerbot".into(),
        modules: SPAWN_MODULES.iter().map(|m| (*m).to_owned()).collect(),
        network: Network::Solo,
        ports: BTreeMap::from([("world".to_owned(), 8086)]),
        db_port: 0,
        created: 0,
        regenerate: Vec::new(),
        build: None,
        source: None,
        options: BTreeMap::new(),
        name: None,
    };
    let enabled = rk_runtime::modules::enabled_modules(&catalog, &record).unwrap();
    let modules: Vec<_> = enabled
        .iter()
        .filter_map(|m| catalog.module("azerothcore", m))
        .collect();
    let spawns: Vec<_> = modules.iter().flat_map(|m| m.spawns.clone()).collect();
    let module_databases: Vec<_> = modules
        .iter()
        .flat_map(|m| m.database.extra_databases.clone())
        .collect();
    assert_eq!(spawns.len(), SPAWN_MODULES.len(), "{enabled:?}");
    let configs = dir.path().join("configs");
    std::fs::create_dir_all(&configs).unwrap();
    std::fs::write(configs.join("worldserver.conf"), "GameType = 1\n").unwrap();
    let cx = DriverContext {
        db: &db,
        emulator: &catalog.emulator("azerothcore").unwrap().manifest,
        record: &record,
        realm_address: Ipv4Addr::LOCALHOST,
        configs: &configs,
        spawns: &spawns,
        module_databases: &module_databases,
    };
    let driver = AzerothCore;

    // First start: no Playerbots tables yet.
    driver.prepare_start(&cx).await.unwrap();
    let realm = "SELECT address, localAddress, port, icon FROM acore_auth.realmlist;";
    assert_eq!(query(&db, realm).await, ["127.0.0.1\t127.0.0.1\t8086\t1"]);
    let placed = "SELECT COUNT(*) FROM acore_world.creature \
                  WHERE guid BETWEEN 9900000 AND 9900999 AND Comment = 'Placed by RealmKeeper';";
    assert_eq!(query(&db, placed).await, ["6"], "3 modules x 2 places");

    // Accounts.
    let player = driver
        .create_account(&cx, &account("Player1", 3))
        .await
        .unwrap();
    let bot = driver
        .create_account(&cx, &account("rndbot1", 0))
        .await
        .unwrap();
    let duplicate = driver.create_account(&cx, &account("PLAYER1", 0)).await;
    assert!(duplicate.is_err(), "names are case-insensitive");
    let found = driver.find_account(&cx, "player1").await.unwrap();
    assert_eq!(found, Some(player));
    assert_eq!(driver.find_account(&cx, "nobody").await.unwrap(), None);
    let links = "SELECT COUNT(*) FROM acore_auth.realmcharacters;";
    assert_eq!(query(&db, links).await, ["2"]);
    let accounts = driver.accounts(&cx).await.unwrap().unwrap();
    assert_eq!(accounts.len(), 1, "bot accounts are not listed");
    assert_eq!(accounts[0].id, player);
    assert_eq!(accounts[0].username, "PLAYER1");
    assert_eq!(accounts[0].gm_level, 3);
    driver.set_gm_level(&cx, "Player1", 0).await.unwrap();
    let accounts = driver.accounts(&cx).await.unwrap().unwrap();
    assert_eq!(accounts[0].gm_level, 0);

    // Characters and population.
    let arthas = NewCharacter {
        account_id: player,
        name: "Arthas".into(),
        race: Some("human".into()),
        class: Some("paladin".into()),
    };
    let guid = driver.create_character(&cx, &arthas).await.unwrap();
    let found = driver.find_character(&cx, "Arthas").await.unwrap();
    assert_eq!(found, Some((player, guid)));
    let sql = format!(
        "INSERT INTO acore_characters.characters \
         (guid, account, name, taximask, innTriggerId, online) \
         VALUES ({}, {bot}, 'Botty', '', 0, 1);\n\
         UPDATE acore_characters.characters SET online = 1 WHERE guid = {guid};",
        guid + 1
    );
    db.execute(&sql).await.unwrap();
    let population = driver.population(&cx).await.unwrap();
    let expected = Population {
        players: 1,
        bots: 1,
    };
    assert_eq!(population, Some(expected));
    let online = driver.online_characters(&cx, 10).await.unwrap().unwrap();
    let online: Vec<_> = online.iter().map(|c| (c.name.as_str(), c.bot)).collect();
    assert_eq!(online, [("Arthas", false), ("Botty", true)]);
    let places = driver.teleport_locations(&cx).await.unwrap();
    assert_eq!(places, ["Orgrimmar", "Stormwind"]);

    // Next start, once worldserver has made the Playerbots tables: the
    // types of the player account and of a deleted account are forgotten.
    db.execute(PLAYERBOTS_SCHEMA).await.unwrap();
    let sql = format!(
        "INSERT INTO acore_playerbots.playerbots_account_type (account_id, account_type) \
         VALUES ({player}, 2), ({bot}, 1), (999, 1);"
    );
    db.execute(&sql).await.unwrap();
    driver.prepare_start(&cx).await.unwrap();
    let types = "SELECT account_id FROM acore_playerbots.playerbots_account_type;";
    assert_eq!(query(&db, types).await, [bot.to_string()]);
    assert_eq!(query(&db, placed).await, ["6"], "spawns are replaced");
    let all = "SELECT COUNT(*) FROM acore_world.creature;";
    assert_eq!(query(&db, all).await, ["7"], "other spawns are kept");
}
