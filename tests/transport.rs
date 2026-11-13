use ataria2_service::{ServiceClient, ServiceServer};
use serde_json::json;

#[tokio::test]
async fn client_handshakes_and_routes_download_methods_to_the_single_server() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    let endpoint = data_directory.join("service.endpoint.json");
    let server = ServiceServer::bind(
        endpoint.clone(),
        data_directory.clone(),
        root.path().join("aria2c"),
        "0.1.0-alpha.0",
    )
    .await
    .unwrap();
    let stop = server.stop_handle();
    let server_task = tokio::spawn(server.run());

    let client = ServiceClient::connect(&endpoint, &data_directory, "transport-test")
        .await
        .unwrap();
    let result = client
        .request("download.tasks.list", json!({}))
        .await
        .unwrap();

    assert_eq!(result, json!({ "tasks": [] }));
    stop.notify_one();
    server_task.await.unwrap().unwrap();
    assert!(!data_directory.join("service.lock").exists());
}

#[tokio::test]
async fn a_second_server_cannot_acquire_the_same_service_lease() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    let endpoint = data_directory.join("service.endpoint.json");
    let first = ServiceServer::bind(
        endpoint.clone(),
        data_directory.clone(),
        root.path().join("aria2c"),
        "0.1.0-alpha.0",
    )
    .await
    .unwrap();

    let error = ServiceServer::bind(
        endpoint,
        data_directory,
        root.path().join("aria2c"),
        "0.1.0-alpha.0",
    )
    .await
    .err()
    .unwrap();

    assert_eq!(error.code, "DOWNLOAD_SERVICE_LEASE_FAILED");
    drop(first);
}

#[tokio::test]
async fn a_stale_lease_file_does_not_block_service_restart() {
    let root = tempfile::tempdir().unwrap();
    let data_directory = root.path().join("aria2");
    std::fs::create_dir_all(&data_directory).unwrap();
    std::fs::write(data_directory.join("service.lock"), "stale-token\n999999\n").unwrap();

    let server = ServiceServer::bind(
        data_directory.join("service.endpoint.json"),
        data_directory,
        root.path().join("aria2c"),
        "0.1.0-alpha.0",
    )
    .await
    .unwrap();

    drop(server);
}
