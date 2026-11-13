use std::{path::PathBuf, time::Duration};

use ataria2_service::DownloadService;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
#[ignore = "requires ATARIA2_REAL_ARIA2"]
async fn pinned_aria2_downloads_a_real_file_through_the_service() {
    let runtime = PathBuf::from(std::env::var_os("ATARIA2_REAL_ARIA2").unwrap());
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let body = b"ataria2-real-download-smoke\n";
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).await.unwrap();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(body).await.unwrap();
    });
    let service = DownloadService::open(root.path(), runtime).unwrap();
    let mut events = service.subscribe();
    let started = service
        .request(
            "download.tasks.start",
            json!({ "request": {
                "url": format!("http://{address}/payload.bin"),
                "downloadDir": downloads,
                "fileName": "payload.bin",
                "connections": 2,
                "split": 2,
                "minSplitSize": "1M"
            }}),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap().to_string();

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.id == id && event.status.as_str() == "completed" {
                break;
            }
        }
    })
    .await
    .unwrap();
    server.await.unwrap();

    assert_eq!(std::fs::read(downloads.join("payload.bin")).unwrap(), body);
}
