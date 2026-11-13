use std::path::PathBuf;

use ataria2_service::{parse_service_args, ServiceError, ServiceServer};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}: {}", error.code, error.message);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ServiceError> {
    let options = parse_service_args(std::env::args_os().skip(1))?;
    let runtime = runtime_path()?;
    let server = ServiceServer::bind(
        options.endpoint_path,
        options.data_directory,
        runtime,
        env!("CARGO_PKG_VERSION"),
    )
    .await?;
    let stop = server.stop_handle();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            stop.notify_one();
        }
    });
    server.run().await
}

fn runtime_path() -> Result<PathBuf, ServiceError> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("ATARIA2_ARIA2_PATH") {
        let path = PathBuf::from(path);
        ensure_runtime_executable(&path)?;
        return Ok(path);
    }
    let executable = std::env::current_exe().map_err(|error| {
        ServiceError::new("DOWNLOAD_SERVICE_RUNTIME_UNAVAILABLE", error.to_string())
    })?;
    let name = if cfg!(windows) {
        "aria2c.exe"
    } else {
        "aria2c"
    };
    let path = executable
        .parent()
        .map(|directory| directory.join(name))
        .ok_or_else(|| {
            ServiceError::new(
                "DOWNLOAD_SERVICE_RUNTIME_UNAVAILABLE",
                "service executable has no parent directory",
            )
        })?;
    ensure_runtime_executable(&path)?;
    Ok(path)
}

fn ensure_runtime_executable(path: &std::path::Path) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path)
            .map_err(|error| {
                ServiceError::new("DOWNLOAD_SERVICE_RUNTIME_UNAVAILABLE", error.to_string())
            })?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            ServiceError::new("DOWNLOAD_SERVICE_RUNTIME_UNAVAILABLE", error.to_string())
        })?;
    }
    Ok(())
}
