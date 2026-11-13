use std::{path::PathBuf, time::Duration};

use ataria2_service::{ServiceSupervisor, ServiceSupervisorError, StandaloneService};
use serde_json::Value;
use tauri::{Emitter, Manager, State};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

#[tauri::command]
async fn connect_download_service(
    app: tauri::AppHandle,
    service: State<'_, StandaloneService>,
) -> Result<(), ServiceSupervisorError> {
    let mut events = service.connect().await?;
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let _ = app.emit("download-service-event", event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Ok(())
}

#[tauri::command]
async fn download_service_request(
    service: State<'_, StandaloneService>,
    method: String,
    params: Value,
) -> Result<Value, ServiceSupervisorError> {
    service.request(method, params).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_directory = shared_data_directory(app)?;
            let endpoint_path = data_directory.join("service.endpoint.json");
            let executable = service_executable(app)?;
            let supervisor = ServiceSupervisor::new(
                endpoint_path,
                data_directory,
                executable,
                "ataria2-standalone",
                env!("CARGO_PKG_VERSION"),
                STARTUP_TIMEOUT,
            );
            app.manage(StandaloneService::new(supervisor));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect_download_service,
            download_service_request
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Aria2 download client");
}

fn shared_data_directory(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("ATARIA2_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    let standalone_directory = app.path().app_data_dir()?;
    let application_data_root = standalone_directory
        .parent()
        .ok_or_else(|| std::io::Error::other("standalone app data directory has no parent"))?;
    Ok(application_data_root
        .join("com.attson.attool")
        .join("aria2"))
}

fn service_executable(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("ATARIA2_SERVICE_EXECUTABLE") {
        return Ok(PathBuf::from(path));
    }
    let executable_name = if cfg!(windows) {
        "ataria2-service.exe"
    } else {
        "ataria2-service"
    };
    Ok(app.path().resource_dir()?.join(executable_name))
}
