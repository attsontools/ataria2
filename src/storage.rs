use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, DatabaseName, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ServiceError;

const DATABASE_NAME: &str = "aria2-tasks.sqlite3";
const MIGRATING_NAME: &str = "aria2-tasks.sqlite3.migrating";
const TAKEOVER_NAME: &str = "takeover.json";
const CAPABILITY_NAME: &str = "takeover-capability-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl DownloadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_database(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub download_dir: String,
    pub file_name: Option<String>,
    pub status: DownloadStatus,
    pub progress: f32,
    pub speed: Option<String>,
    pub eta: Option<String>,
    pub message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub local_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEventPayload {
    pub id: String,
    pub status: DownloadStatus,
    pub progress: f32,
    pub speed: Option<String>,
    pub eta: Option<String>,
    pub message: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub local_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    Created,
    Existing,
    Migrated { tasks: u64 },
}

#[derive(Clone)]
pub struct TaskStore {
    db_path: PathBuf,
}

impl TaskStore {
    pub fn open(data_directory: &Path) -> Result<(Self, MigrationOutcome), ServiceError> {
        fs::create_dir_all(data_directory).map_err(storage_io_error)?;
        let db_path = data_directory.join(DATABASE_NAME);
        let outcome = if db_path.exists() {
            validate_database(&db_path)?;
            remove_if_exists(&data_directory.join(MIGRATING_NAME))?;
            write_takeover(data_directory, "owned", None)?;
            MigrationOutcome::Existing
        } else {
            migrate_or_create(data_directory, &db_path)?
        };
        let store = Self { db_path };
        store.mark_interrupted_tasks()?;
        Ok((store, outcome))
    }

    pub fn list_tasks(&self) -> Result<Vec<DownloadTask>, ServiceError> {
        let connection = Connection::open(&self.db_path).map_err(storage_error)?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, url, download_dir, file_name, status, progress, speed, eta, message,
                       created_at, started_at, finished_at, local_path
                FROM download_tasks
                ORDER BY datetime(created_at) DESC, id DESC
                LIMIT 300
                "#,
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                let status: String = row.get(4)?;
                Ok(DownloadTask {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    download_dir: row.get(2)?,
                    file_name: row.get(3)?,
                    status: DownloadStatus::from_database(&status),
                    progress: row.get(5)?,
                    speed: row.get(6)?,
                    eta: row.get(7)?,
                    message: row.get(8)?,
                    created_at: row.get(9)?,
                    started_at: row.get(10)?,
                    finished_at: row.get(11)?,
                    local_path: row.get(12)?,
                })
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    pub fn create_task(
        &self,
        url: &str,
        download_dir: &str,
        file_name: Option<&str>,
    ) -> Result<String, ServiceError> {
        let mut connection = Connection::open(&self.db_path).map_err(storage_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let mut statement = transaction
            .prepare("SELECT id FROM download_tasks")
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        let mut maximum = 0_u64;
        for row in rows {
            if let Some(number) = row
                .map_err(storage_error)?
                .strip_prefix("dl-")
                .and_then(|value| value.parse::<u64>().ok())
            {
                maximum = maximum.max(number);
            }
        }
        drop(statement);
        let id = format!("dl-{}", maximum + 1);
        transaction
            .execute(
                r#"
                INSERT INTO download_tasks (
                    id, url, download_dir, file_name, status, progress, message
                ) VALUES (?1, ?2, ?3, ?4, 'queued', 0, 'aria2 任务已创建')
                "#,
                params![id, url, download_dir, file_name],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(id)
    }

    pub fn persist_event(&self, event: &DownloadEventPayload) -> Result<(), ServiceError> {
        let connection = Connection::open(&self.db_path).map_err(storage_error)?;
        connection
            .execute(
                r#"
                UPDATE download_tasks
                SET status = ?2,
                    progress = CASE
                        WHEN ?3 > 0 OR ?2 IN ('queued', 'completed') THEN ?3
                        ELSE progress
                    END,
                    speed = ?4,
                    eta = ?5,
                    message = CASE WHEN ?6 IS NOT NULL THEN ?6 ELSE message END,
                    started_at = CASE
                        WHEN started_at IS NULL AND ?2 = 'running' THEN datetime('now', 'localtime')
                        ELSE started_at
                    END,
                    finished_at = CASE
                        WHEN finished_at IS NULL AND ?2 IN ('completed', 'failed', 'cancelled')
                            THEN datetime('now', 'localtime')
                        ELSE finished_at
                    END,
                    local_path = COALESCE(local_path, ?7),
                    updated_at = datetime('now', 'localtime')
                WHERE id = ?1
                "#,
                params![
                    event.id,
                    event.status.as_str(),
                    event.progress,
                    event.speed,
                    event.eta,
                    event.message,
                    event.local_path,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn clear_completed(&self) -> Result<u64, ServiceError> {
        let connection = Connection::open(&self.db_path).map_err(storage_error)?;
        connection
            .execute("DELETE FROM download_tasks WHERE status = 'completed'", [])
            .map(|count| count as u64)
            .map_err(storage_error)
    }

    pub fn delete_task(&self, id: &str) -> Result<(), ServiceError> {
        let connection = Connection::open(&self.db_path).map_err(storage_error)?;
        let status = connection
            .query_row(
                "SELECT status FROM download_tasks WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| {
                ServiceError::new(
                    "DOWNLOAD_SERVICE_TASK_NOT_FOUND",
                    "download task was not found",
                )
            })?;
        if status == "queued" || status == "running" {
            return Err(ServiceError::new(
                "DOWNLOAD_SERVICE_TASK_ACTIVE",
                "active download task must be cancelled before deletion",
            ));
        }
        connection
            .execute("DELETE FROM download_tasks WHERE id = ?1", params![id])
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn download_directory(&self, id: &str) -> Result<PathBuf, ServiceError> {
        let connection = Connection::open(&self.db_path).map_err(storage_error)?;
        connection
            .query_row(
                "SELECT download_dir FROM download_tasks WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(PathBuf::from)
            .ok_or_else(|| {
                ServiceError::new(
                    "DOWNLOAD_SERVICE_TASK_NOT_FOUND",
                    "download task was not found",
                )
            })
    }

    fn mark_interrupted_tasks(&self) -> Result<(), ServiceError> {
        let connection = Connection::open(&self.db_path).map_err(storage_error)?;
        connection
            .execute(
                r#"
                UPDATE download_tasks
                SET status = 'failed', speed = NULL, eta = NULL,
                    message = '应用已退出，任务未继续运行',
                    finished_at = COALESCE(finished_at, datetime('now', 'localtime')),
                    updated_at = datetime('now', 'localtime')
                WHERE status IN ('queued', 'running')
                "#,
                [],
            )
            .map_err(storage_error)?;
        Ok(())
    }
}

fn migrate_or_create(
    data_directory: &Path,
    db_path: &Path,
) -> Result<MigrationOutcome, ServiceError> {
    let takeover = takeover_state(data_directory)?;
    if takeover.as_deref() == Some("owned") {
        return Err(ServiceError::new(
            "DOWNLOAD_SERVICE_DATA_MISSING",
            "service owns download history but its database is missing",
        ));
    }
    if takeover.as_deref() == Some("migrating") {
        remove_if_exists(&data_directory.join(MIGRATING_NAME))?;
        remove_if_exists(&data_directory.join(TAKEOVER_NAME))?;
    }
    let legacy = data_directory
        .parent()
        .ok_or_else(|| migration_error("data directory has no parent"))?
        .join(DATABASE_NAME);
    if !legacy.exists() {
        init_database(db_path)?;
        validate_database(db_path)?;
        write_takeover(data_directory, "owned", None)?;
        return Ok(MigrationOutcome::Created);
    }
    if !data_directory.join(CAPABILITY_NAME).exists() {
        return Err(ServiceError::new(
            "DOWNLOAD_SERVICE_HOST_UPGRADE_REQUIRED",
            "run a takeover-capable AT Tool version before migrating download history",
        ));
    }

    let nonce = Uuid::new_v4().to_string();
    create_migrating_marker(data_directory, &legacy, &nonce)?;
    let result = migrate_legacy(data_directory, &legacy, db_path, &nonce);
    if result.is_err() {
        let _ = fs::remove_file(data_directory.join(TAKEOVER_NAME));
        let _ = fs::remove_file(data_directory.join(MIGRATING_NAME));
    }
    result
}

fn takeover_state(data_directory: &Path) -> Result<Option<String>, ServiceError> {
    let path = data_directory.join(TAKEOVER_NAME);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(migration_io_error)?;
    let marker: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| migration_error(format!("cannot decode takeover marker: {error}")))?;
    Ok(marker
        .get("state")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

fn migrate_legacy(
    data_directory: &Path,
    legacy: &Path,
    db_path: &Path,
    nonce: &str,
) -> Result<MigrationOutcome, ServiceError> {
    let mut source = Connection::open(legacy).map_err(migration_sql_error)?;
    let source_transaction = source
        .transaction_with_behavior(TransactionBehavior::Exclusive)
        .map_err(migration_sql_error)?;
    let active: i64 = source_transaction
        .query_row(
            "SELECT COUNT(*) FROM download_tasks WHERE status IN ('queued', 'running')",
            [],
            |row| row.get(0),
        )
        .map_err(migration_sql_error)?;
    if active > 0 {
        return Err(ServiceError::new(
            "DOWNLOAD_SERVICE_TAKEOVER_BUSY",
            "legacy download tasks are still active",
        ));
    }
    let source_count: i64 = source_transaction
        .query_row("SELECT COUNT(*) FROM download_tasks", [], |row| row.get(0))
        .map_err(migration_sql_error)?;
    source_transaction.commit().map_err(migration_sql_error)?;
    let migrating = data_directory.join(MIGRATING_NAME);
    let _ = fs::remove_file(&migrating);
    source
        .backup(DatabaseName::Main, &migrating, None)
        .map_err(migration_sql_error)?;
    drop(source);

    let connection = Connection::open(&migrating).map_err(migration_sql_error)?;
    migrate_schema(&connection)?;
    connection
        .execute_batch("DROP TABLE IF EXISTS logo_presets;")
        .map_err(migration_sql_error)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(migration_sql_error)?;
    if integrity != "ok" {
        return Err(migration_error("migrated database failed integrity check"));
    }
    let target_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM download_tasks", [], |row| row.get(0))
        .map_err(migration_sql_error)?;
    if target_count != source_count {
        return Err(migration_error(
            "migrated task count does not match the source",
        ));
    }
    drop(connection);
    sync_file(&migrating)?;
    fs::rename(&migrating, db_path).map_err(migration_io_error)?;
    write_takeover(data_directory, "owned", Some((legacy, nonce)))?;
    Ok(MigrationOutcome::Migrated {
        tasks: target_count as u64,
    })
}

fn init_database(path: &Path) -> Result<(), ServiceError> {
    let connection = Connection::open(path).map_err(storage_error)?;
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS download_tasks (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                download_dir TEXT NOT NULL,
                file_name TEXT,
                status TEXT NOT NULL,
                progress REAL NOT NULL DEFAULT 0,
                speed TEXT,
                eta TEXT,
                message TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
                started_at TEXT,
                finished_at TEXT,
                local_path TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_download_tasks_created_at
                ON download_tasks(created_at DESC);
            "#,
        )
        .map_err(storage_error)
}

fn migrate_schema(connection: &Connection) -> Result<(), ServiceError> {
    for statement in [
        "ALTER TABLE download_tasks ADD COLUMN started_at TEXT",
        "ALTER TABLE download_tasks ADD COLUMN finished_at TEXT",
        "ALTER TABLE download_tasks ADD COLUMN local_path TEXT",
    ] {
        if let Err(error) = connection.execute(statement, []) {
            if !error.to_string().contains("duplicate column name") {
                return Err(migration_sql_error(error));
            }
        }
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_download_tasks_created_at ON download_tasks(created_at DESC)",
            [],
        )
        .map_err(migration_sql_error)?;
    Ok(())
}

fn validate_database(path: &Path) -> Result<(), ServiceError> {
    let connection = Connection::open(path).map_err(storage_error)?;
    migrate_schema(&connection)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(storage_error)?;
    if integrity != "ok" {
        return Err(ServiceError::new(
            "DOWNLOAD_SERVICE_DATA_MISSING",
            "service database failed integrity check",
        ));
    }
    Ok(())
}

fn create_migrating_marker(
    data_directory: &Path,
    source: &Path,
    nonce: &str,
) -> Result<(), ServiceError> {
    let path = data_directory.join(TAKEOVER_NAME);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(migration_io_error)?;
    let bytes = takeover_bytes("migrating", source, nonce)?;
    file.write_all(&bytes).map_err(migration_io_error)?;
    file.sync_all().map_err(migration_io_error)
}

fn write_takeover(
    data_directory: &Path,
    state: &str,
    source_and_nonce: Option<(&Path, &str)>,
) -> Result<(), ServiceError> {
    let source = source_and_nonce
        .map(|(source, _)| source)
        .unwrap_or_else(|| Path::new(""));
    let nonce = source_and_nonce.map(|(_, nonce)| nonce).unwrap_or("");
    let destination = data_directory.join(TAKEOVER_NAME);
    let bytes = takeover_bytes(state, source, nonce)?;
    if destination.exists() {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&destination)
            .map_err(migration_io_error)?;
        file.write_all(&bytes).map_err(migration_io_error)?;
        return file.sync_all().map_err(migration_io_error);
    }
    let temporary = data_directory.join(format!("takeover.tmp-{}", Uuid::new_v4()));
    fs::write(&temporary, bytes).map_err(migration_io_error)?;
    sync_file(&temporary)?;
    fs::rename(&temporary, destination).map_err(migration_io_error)
}

fn takeover_bytes(state: &str, source: &Path, nonce: &str) -> Result<Vec<u8>, ServiceError> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "protocolVersion": 1,
        "state": state,
        "nonce": nonce,
        "source": source.to_string_lossy(),
    }))
    .map_err(|error| migration_error(format!("cannot encode takeover marker: {error}")))
}

fn sync_file(path: &Path) -> Result<(), ServiceError> {
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(migration_io_error)
}

fn remove_if_exists(path: &Path) -> Result<(), ServiceError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(migration_io_error(error)),
    }
}

fn storage_error(error: rusqlite::Error) -> ServiceError {
    ServiceError::new("DOWNLOAD_SERVICE_STORAGE_FAILED", error.to_string())
}

fn storage_io_error(error: std::io::Error) -> ServiceError {
    ServiceError::new("DOWNLOAD_SERVICE_STORAGE_FAILED", error.to_string())
}

fn migration_sql_error(error: rusqlite::Error) -> ServiceError {
    migration_error(error.to_string())
}

fn migration_io_error(error: std::io::Error) -> ServiceError {
    migration_error(error.to_string())
}

fn migration_error(message: impl Into<String>) -> ServiceError {
    ServiceError::new("DOWNLOAD_SERVICE_MIGRATION_FAILED", message)
}
