use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ServiceError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub service_version: String,
}

pub fn write_endpoint(path: &Path, endpoint: &Endpoint) -> Result<(), ServiceError> {
    if endpoint.port == 0
        || endpoint.token.trim().is_empty()
        || endpoint.service_version.trim().is_empty()
    {
        return Err(endpoint_error("endpoint fields are invalid"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| endpoint_error("endpoint path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| endpoint_error(error.to_string()))?;
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let bytes = serde_json::to_vec(endpoint).map_err(|error| endpoint_error(error.to_string()))?;
    fs::write(&temporary, bytes).map_err(|error| endpoint_error(error.to_string()))?;
    set_private_permissions(&temporary)?;
    fs::rename(&temporary, path).map_err(|error| endpoint_error(error.to_string()))
}

pub fn read_endpoint(path: &Path) -> Result<Endpoint, ServiceError> {
    let bytes = fs::read(path).map_err(|error| {
        ServiceError::new("DOWNLOAD_SERVICE_ENDPOINT_READ_FAILED", error.to_string())
    })?;
    let endpoint: Endpoint = serde_json::from_slice(&bytes)
        .map_err(|error| endpoint_error(format!("cannot decode endpoint: {error}")))?;
    if endpoint.port == 0
        || endpoint.token.trim().is_empty()
        || endpoint.service_version.trim().is_empty()
    {
        return Err(endpoint_error("endpoint fields are invalid"));
    }
    Ok(endpoint)
}

fn endpoint_error(message: impl Into<String>) -> ServiceError {
    ServiceError::new("DOWNLOAD_SERVICE_ENDPOINT_INVALID", message)
}

fn set_private_permissions(path: &Path) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| endpoint_error(error.to_string()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|error| endpoint_error(error.to_string()))?;
    }
    Ok(())
}
