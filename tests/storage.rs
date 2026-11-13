use std::fs;

use ataria2_service::{MigrationOutcome, TaskStore};
use rusqlite::{params, Connection, TransactionBehavior};
use std::collections::HashSet;

#[test]
fn legacy_history_is_migrated_into_the_service_owned_database() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    fs::write(data_directory.join("takeover-capability-v1"), b"1\n").unwrap();
    let legacy = root.path().join("aria2-tasks.sqlite3");
    let connection = Connection::open(&legacy).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE download_tasks (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                download_dir TEXT NOT NULL,
                file_name TEXT,
                status TEXT NOT NULL,
                progress REAL NOT NULL DEFAULT 0,
                speed TEXT,
                eta TEXT,
                message TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                local_path TEXT
            );
            INSERT INTO download_tasks VALUES (
                'dl-7', 'https://example.com/archive.zip', '/tmp/downloads', 'archive.zip',
                'completed', 100, NULL, NULL, '下载完成', '2026-11-01 10:00:00',
                '2026-11-01 10:01:00', '2026-11-01 10:00:01', '2026-11-01 10:01:00',
                '/tmp/downloads/archive.zip'
            );
            CREATE TABLE logo_presets (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            INSERT INTO logo_presets(name) VALUES ('legacy');
            "#,
        )
        .unwrap();
    drop(connection);

    let (store, outcome) = TaskStore::open(&data_directory).unwrap();

    assert_eq!(outcome, MigrationOutcome::Migrated { tasks: 1 });
    let tasks = store.list_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "dl-7");
    assert_eq!(
        tasks[0].local_path.as_deref(),
        Some("/tmp/downloads/archive.zip")
    );
    assert!(legacy.exists());
    assert!(data_directory.join("takeover.json").exists());
    let service_db = Connection::open(data_directory.join("aria2-tasks.sqlite3")).unwrap();
    let logo_table_count: i64 = service_db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'logo_presets'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(logo_table_count, 0);
}

#[test]
fn owned_marker_without_service_database_never_reimports_legacy_data() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    fs::write(data_directory.join("takeover-capability-v1"), b"1\n").unwrap();
    fs::write(
        data_directory.join("takeover.json"),
        br#"{"protocolVersion":1,"state":"owned","nonce":"n","source":"legacy"}"#,
    )
    .unwrap();
    let legacy = root.path().join("aria2-tasks.sqlite3");
    let connection = Connection::open(legacy).unwrap();
    connection
        .execute_batch("CREATE TABLE download_tasks (id TEXT PRIMARY KEY, status TEXT NOT NULL);")
        .unwrap();

    let error = TaskStore::open(&data_directory).err().unwrap();

    assert_eq!(error.code, "DOWNLOAD_SERVICE_DATA_MISSING");
}

#[test]
fn task_ids_are_allocated_atomically_across_concurrent_writers() {
    let root = tempfile::tempdir().unwrap();
    let (store, _) = TaskStore::open(root.path()).unwrap();
    let handles = (0..8)
        .map(|index| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .create_task(&format!("https://example.com/{index}.zip"), "/tmp", None)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();

    let ids = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<HashSet<_>>();

    assert_eq!(ids.len(), 8);
    assert_eq!(store.list_tasks().unwrap().len(), 8);
}

#[test]
fn takeover_waits_for_an_inflight_legacy_write_and_migrates_its_committed_task() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    fs::write(data_directory.join("takeover-capability-v1"), b"1\n").unwrap();
    let legacy = root.path().join("aria2-tasks.sqlite3");
    create_legacy_database(&legacy, "completed");

    let mut writer = Connection::open(&legacy).unwrap();
    let transaction = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute(
            "INSERT INTO download_tasks (id, url, download_dir, status, created_at, updated_at) VALUES (?1, ?2, '/tmp', 'completed', '2026-01-02', '2026-01-02')",
            params!["dl-2", "https://example.com/b"],
        )
        .unwrap();

    let service_directory = data_directory.clone();
    let migration = std::thread::spawn(move || TaskStore::open(&service_directory));
    let marker = data_directory.join("takeover.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !marker.exists() {
        assert!(
            !migration.is_finished(),
            "migration finished before taking the source lock"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "migration marker was not created"
        );
        std::thread::yield_now();
    }

    transaction.commit().unwrap();
    let (store, outcome) = migration.join().unwrap().unwrap();

    assert_eq!(outcome, MigrationOutcome::Migrated { tasks: 2 });
    let ids = store
        .list_tasks()
        .unwrap()
        .into_iter()
        .map(|task| task.id)
        .collect::<HashSet<_>>();
    assert_eq!(ids, HashSet::from(["dl-1".to_string(), "dl-2".to_string()]));
}

#[test]
fn legacy_history_requires_a_takeover_capable_host() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    create_legacy_database(&root.path().join("aria2-tasks.sqlite3"), "completed");

    let error = TaskStore::open(&data_directory).err().unwrap();

    assert_eq!(error.code, "DOWNLOAD_SERVICE_HOST_UPGRADE_REQUIRED");
}

#[test]
fn active_legacy_tasks_block_takeover_without_changing_the_source() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    fs::write(data_directory.join("takeover-capability-v1"), b"1\n").unwrap();
    let legacy = root.path().join("aria2-tasks.sqlite3");
    create_legacy_database(&legacy, "running");

    let error = TaskStore::open(&data_directory).err().unwrap();

    assert_eq!(error.code, "DOWNLOAD_SERVICE_TAKEOVER_BUSY");
    assert!(!data_directory.join("takeover.json").exists());
    let connection = Connection::open(legacy).unwrap();
    let status: String = connection
        .query_row("SELECT status FROM download_tasks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(status, "running");
}

#[test]
fn interrupted_migration_is_retried_from_the_preserved_legacy_database() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    fs::write(data_directory.join("takeover-capability-v1"), b"1\n").unwrap();
    let legacy = root.path().join("aria2-tasks.sqlite3");
    create_legacy_database(&legacy, "completed");
    fs::write(
        data_directory.join("aria2-tasks.sqlite3.migrating"),
        b"partial",
    )
    .unwrap();
    fs::write(
        data_directory.join("takeover.json"),
        format!(
            r#"{{"protocolVersion":1,"state":"migrating","nonce":"old","source":"{}"}}"#,
            legacy.display()
        ),
    )
    .unwrap();

    let (store, outcome) = TaskStore::open(&data_directory).unwrap();

    assert_eq!(outcome, MigrationOutcome::Migrated { tasks: 1 });
    assert_eq!(store.list_tasks().unwrap()[0].id, "dl-1");
    assert!(!data_directory
        .join("aria2-tasks.sqlite3.migrating")
        .exists());
}

#[test]
fn corrupt_legacy_database_is_preserved_and_never_promoted() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    fs::create_dir_all(&data_directory).unwrap();
    fs::write(data_directory.join("takeover-capability-v1"), b"1\n").unwrap();
    let legacy = root.path().join("aria2-tasks.sqlite3");
    fs::write(&legacy, b"not a sqlite database").unwrap();

    let error = TaskStore::open(&data_directory).err().unwrap();

    assert_eq!(error.code, "DOWNLOAD_SERVICE_MIGRATION_FAILED");
    assert_eq!(fs::read(legacy).unwrap(), b"not a sqlite database");
    assert!(!data_directory.join("aria2-tasks.sqlite3").exists());
    assert!(!data_directory.join("takeover.json").exists());
}

#[test]
fn existing_service_database_wins_over_stale_migration_output() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    let (store, _) = TaskStore::open(&data_directory).unwrap();
    store
        .create_task("https://example.com/kept", "/tmp", None)
        .unwrap();
    fs::write(
        data_directory.join("aria2-tasks.sqlite3.migrating"),
        b"stale",
    )
    .unwrap();

    let (reopened, outcome) = TaskStore::open(&data_directory).unwrap();

    assert_eq!(outcome, MigrationOutcome::Existing);
    assert_eq!(reopened.list_tasks().unwrap().len(), 1);
    assert!(!data_directory
        .join("aria2-tasks.sqlite3.migrating")
        .exists());
}

fn create_legacy_database(path: &std::path::Path, status: &str) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE download_tasks (
                id TEXT PRIMARY KEY, url TEXT NOT NULL, download_dir TEXT NOT NULL,
                file_name TEXT, status TEXT NOT NULL, progress REAL NOT NULL DEFAULT 0,
                speed TEXT, eta TEXT, message TEXT, created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO download_tasks (id, url, download_dir, status, created_at, updated_at) VALUES ('dl-1', 'https://example.com/a', '/tmp', ?1, '2026-01-01', '2026-01-01')",
            [status],
        )
        .unwrap();
}
