use ataria2_service::{negotiate, HandshakeRequest, APP_ID, DATA_API, HOST_API, SERVICE_API};

#[test]
fn compatible_client_negotiates_download_service_capability() {
    let request = HandshakeRequest {
        protocol_version: 1,
        app_id: "com.attson.aria2".into(),
        client_id: "plugin-test".into(),
        request_id: "request-1".into(),
        host_api: "2.x".into(),
        data_api: "1.0.0".into(),
        service_api: "1.x".into(),
        data_directory: "/tmp/ataria2".into(),
        token: "secret".into(),
    };

    let response = negotiate(&request, "secret", "/tmp/ataria2").unwrap();

    assert_eq!(APP_ID, "com.attson.aria2");
    assert_eq!(HOST_API, "2.0.0");
    assert_eq!(DATA_API, "1.0.0");
    assert_eq!(SERVICE_API, "1.0.0");
    assert_eq!(response.request_id, "request-1");
    assert!(response.capabilities.download_tasks);
}
