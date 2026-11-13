use std::{ffi::OsString, path::PathBuf};

use ataria2_service::parse_service_args;

#[test]
fn service_requires_explicit_endpoint_and_data_directory() {
    let options = parse_service_args([
        OsString::from("--endpoint"),
        OsString::from("/tmp/ataria2/endpoint.json"),
        OsString::from("--data-directory"),
        OsString::from("/tmp/ataria2/data"),
    ])
    .unwrap();

    assert_eq!(
        options.endpoint_path,
        PathBuf::from("/tmp/ataria2/endpoint.json")
    );
    assert_eq!(options.data_directory, PathBuf::from("/tmp/ataria2/data"));
}
