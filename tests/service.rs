use ataria2_service::DownloadService;
use serde_json::json;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn task_list_is_exposed_through_the_public_service_method() {
    let root = tempfile::tempdir().unwrap();
    let service = DownloadService::open(root.path(), root.path().join("aria2c")).unwrap();

    let result = service
        .request("download.tasks.list", json!({}))
        .await
        .unwrap();

    assert_eq!(result, json!({ "tasks": [] }));
}

#[tokio::test]
async fn default_download_directory_is_exposed_through_the_service() {
    let root = tempfile::tempdir().unwrap();
    let service = DownloadService::open(root.path(), root.path().join("aria2c")).unwrap();

    let result = service
        .request("download.defaultDirectory", json!({}))
        .await
        .unwrap();

    assert!(result["path"].as_str().is_some_and(|path| !path.is_empty()));
}

#[cfg(unix)]
#[tokio::test]
async fn starting_a_download_persists_and_broadcasts_terminal_state() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("aria2c");
    std::fs::write(
        &runtime,
        "#!/bin/sh\nprintf '[#1 1MiB/2MiB(50%%) CN:4 DL:2MiB ETA:1s]\\r'\nprintf '11/14 10:00:00 [NOTICE] Download complete: /tmp/downloads/file.zip\\r'\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&runtime).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions).unwrap();
    let service = DownloadService::open(root.path(), runtime).unwrap();
    let mut events = service.subscribe();

    let started = service
        .request(
            "download.tasks.start",
            json!({
                "request": {
                    "url": "https://example.com/file.zip",
                    "downloadDir": "/tmp/downloads",
                    "fileName": "file.zip",
                    "connections": 4,
                    "split": 8,
                    "minSplitSize": "4M"
                }
            }),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap().to_string();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.id == id && event.status.as_str() == "completed" {
                break event;
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(completed.progress, 100.0);
    assert_eq!(
        completed.local_path.as_deref(),
        Some("/tmp/downloads/file.zip")
    );
    let listed = service
        .request("download.tasks.list", json!({}))
        .await
        .unwrap();
    assert_eq!(listed["tasks"][0]["status"], "completed");
    assert_eq!(listed["tasks"][0]["progress"], 100.0);
}

#[cfg(unix)]
#[tokio::test]
async fn running_download_can_be_cancelled_through_the_service() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("aria2c");
    std::fs::write(&runtime, "#!/bin/sh\nsleep 30\n").unwrap();
    let mut permissions = std::fs::metadata(&runtime).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions).unwrap();
    let service = DownloadService::open(root.path(), runtime).unwrap();
    let mut events = service.subscribe();
    let started = service
        .request(
            "download.tasks.start",
            json!({ "request": { "url": "https://example.com/large.iso", "downloadDir": "/tmp" } }),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap();

    let result = service
        .request("download.tasks.cancel", json!({ "id": id }))
        .await
        .unwrap();

    assert_eq!(result, json!({ "cancelled": true }));
    let cancelled = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.id == id && event.status.as_str() == "cancelled" {
                break event;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(cancelled.message.as_deref(), Some("已取消下载任务"));
}

#[cfg(unix)]
#[tokio::test]
async fn completed_downloads_can_be_cleared_from_history() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("aria2c");
    std::fs::write(&runtime, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&runtime).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions).unwrap();
    let service = DownloadService::open(root.path(), runtime).unwrap();
    let mut events = service.subscribe();
    let started = service
        .request(
            "download.tasks.start",
            json!({ "request": { "url": "https://example.com/done.zip", "downloadDir": "/tmp" } }),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap().to_string();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.id == id && event.status.as_str() == "completed" {
                break;
            }
        }
    })
    .await
    .unwrap();

    let result = service
        .request("download.tasks.clearCompleted", json!({}))
        .await
        .unwrap();

    assert_eq!(result, json!({ "count": 1 }));
    assert_eq!(
        service
            .request("download.tasks.list", json!({}))
            .await
            .unwrap(),
        json!({ "tasks": [] })
    );
}

#[cfg(unix)]
#[tokio::test]
async fn task_must_be_terminal_before_it_can_be_deleted() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("aria2c");
    std::fs::write(&runtime, "#!/bin/sh\nsleep 30\n").unwrap();
    let mut permissions = std::fs::metadata(&runtime).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions).unwrap();
    let service = DownloadService::open(root.path(), runtime).unwrap();
    let mut events = service.subscribe();
    let started = service
        .request(
            "download.tasks.start",
            json!({ "request": { "url": "https://example.com/delete.iso", "downloadDir": "/tmp" } }),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap();

    let error = service
        .request("download.tasks.delete", json!({ "id": id }))
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "DOWNLOAD_SERVICE_TASK_ACTIVE");
    service
        .request("download.tasks.cancel", json!({ "id": id }))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.id == id && event.status.as_str() == "cancelled" {
                break;
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(
        service
            .request("download.tasks.delete", json!({ "id": id }))
            .await
            .unwrap(),
        json!({ "deleted": true })
    );
}
