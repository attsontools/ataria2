use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use fs2::FileExt;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{tcp::OwnedWriteHalf, TcpListener, TcpStream},
    sync::{Mutex, Notify},
};
use uuid::Uuid;

use crate::{
    negotiate, write_endpoint, DownloadService, Endpoint, ErrorEnvelope, ServiceError,
    ServiceHandshake, ServiceReply, ServiceRequest,
};

const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

pub struct ServiceServer {
    listener: TcpListener,
    endpoint_path: PathBuf,
    data_directory: PathBuf,
    token: String,
    service: DownloadService,
    stop: Arc<Notify>,
    _lease: ServiceLease,
}

impl ServiceServer {
    pub async fn bind(
        endpoint_path: PathBuf,
        data_directory: PathBuf,
        runtime_path: PathBuf,
        service_version: impl Into<String>,
    ) -> Result<Self, ServiceError> {
        fs::create_dir_all(&data_directory).map_err(lease_error)?;
        let token = Uuid::new_v4().to_string();
        let lease = ServiceLease::acquire(&data_directory, &token)?;
        let service = DownloadService::open(&data_directory, runtime_path)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| {
            ServiceError::new("DOWNLOAD_SERVICE_START_FAILED", error.to_string())
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| ServiceError::new("DOWNLOAD_SERVICE_START_FAILED", error.to_string()))?
            .port();
        write_endpoint(
            &endpoint_path,
            &Endpoint {
                port,
                token: token.clone(),
                service_version: service_version.into(),
            },
        )?;
        Ok(Self {
            listener,
            endpoint_path,
            data_directory,
            token,
            service,
            stop: Arc::new(Notify::new()),
            _lease: lease,
        })
    }

    pub fn stop_handle(&self) -> Arc<Notify> {
        Arc::clone(&self.stop)
    }

    pub async fn run(self) -> Result<(), ServiceError> {
        loop {
            tokio::select! {
                _ = self.stop.notified() => break,
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.map_err(|error| ServiceError::new("DOWNLOAD_SERVICE_ACCEPT_FAILED", error.to_string()))?;
                    let service = self.service.clone();
                    let token = self.token.clone();
                    let data_directory = self.data_directory.to_string_lossy().into_owned();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, service, token, data_directory).await;
                    });
                }
            }
        }
        let _ = fs::remove_file(&self.endpoint_path);
        Ok(())
    }
}

async fn handle_connection(
    stream: TcpStream,
    service: DownloadService,
    token: String,
    data_directory: String,
) -> Result<(), ServiceError> {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let writer = Arc::new(Mutex::new(writer));
    let mut line = Vec::new();
    read_line(&mut reader, &mut line).await?;
    let handshake = serde_json::from_slice::<ServiceHandshake>(&line).map_err(|error| {
        ServiceError::new("DOWNLOAD_SERVICE_REQUEST_INVALID", error.to_string())
    })?;
    let request = match handshake {
        ServiceHandshake::Request(request) => request,
        ServiceHandshake::Response(_) => {
            return Err(ServiceError::new(
                "DOWNLOAD_SERVICE_REQUEST_INVALID",
                "expected handshake request",
            ));
        }
    };
    let response = match negotiate(&request, &token, &data_directory) {
        Ok(response) => ServiceReply::Handshake(response),
        Err(error) => {
            let reply = ServiceReply::Error {
                request_id: request.request_id,
                error: error.into(),
            };
            write_reply(&writer, &reply).await?;
            return Ok(());
        }
    };
    write_reply(&writer, &response).await?;

    let mut events = service.subscribe();
    let event_writer = Arc::clone(&writer);
    let event_task = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if write_reply(&event_writer, &ServiceReply::Event { event })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    loop {
        line.clear();
        match read_line(&mut reader, &mut line).await {
            Ok(false) => break,
            Ok(true) => {}
            Err(error) => {
                event_task.abort();
                return Err(error);
            }
        }
        let request = match serde_json::from_slice::<ServiceRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                let reply = ServiceReply::Error {
                    request_id: String::new(),
                    error: ErrorEnvelope {
                        code: "DOWNLOAD_SERVICE_REQUEST_INVALID".into(),
                        message: error.to_string(),
                        retryable: false,
                    },
                };
                write_reply(&writer, &reply).await?;
                continue;
            }
        };
        let ServiceRequest::Request {
            request_id,
            method,
            params,
        } = request;
        let reply = match service.request(&method, params).await {
            Ok(result) => ServiceReply::Response { request_id, result },
            Err(error) => ServiceReply::Error {
                request_id,
                error: error.into(),
            },
        };
        write_reply(&writer, &reply).await?;
    }
    event_task.abort();
    Ok(())
}

async fn read_line<R>(reader: &mut R, line: &mut Vec<u8>) -> Result<bool, ServiceError>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let available = reader.fill_buf().await.map_err(|error| {
            ServiceError::new("DOWNLOAD_SERVICE_DISCONNECTED", error.to_string())
        })?;
        if available.is_empty() {
            return Ok(!line.is_empty());
        }
        let chunk_len = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + 1)
            .unwrap_or(available.len());
        let remaining = MAX_LINE_BYTES.saturating_add(1).saturating_sub(line.len());
        let consumed = chunk_len.min(remaining);
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if line.len() > MAX_LINE_BYTES {
            return Err(ServiceError::new(
                "DOWNLOAD_SERVICE_REQUEST_TOO_LARGE",
                "service message exceeds 8 MiB",
            ));
        }
        if consumed == chunk_len && line.last() == Some(&b'\n') {
            return Ok(true);
        }
    }
}

async fn write_reply(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    reply: &ServiceReply,
) -> Result<(), ServiceError> {
    let mut bytes = serde_json::to_vec(reply).map_err(|error| {
        ServiceError::new("DOWNLOAD_SERVICE_RESPONSE_INVALID", error.to_string())
    })?;
    bytes.push(b'\n');
    writer
        .lock()
        .await
        .write_all(&bytes)
        .await
        .map_err(|error| ServiceError::new("DOWNLOAD_SERVICE_DISCONNECTED", error.to_string()))
}

struct ServiceLease {
    path: PathBuf,
    token: String,
    file: File,
}

impl ServiceLease {
    fn acquire(data_directory: &Path, token: &str) -> Result<Self, ServiceError> {
        let path = data_directory.join("service.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                ServiceError::new(
                    "DOWNLOAD_SERVICE_LEASE_FAILED",
                    format!("cannot acquire service lease: {error}"),
                )
            })?;
        FileExt::try_lock_exclusive(&file).map_err(|error| {
            ServiceError::new(
                "DOWNLOAD_SERVICE_LEASE_FAILED",
                format!("cannot acquire service lease: {error}"),
            )
        })?;
        file.set_len(0).map_err(lease_error)?;
        writeln!(file, "{token}\n{}", std::process::id()).map_err(lease_error)?;
        file.sync_all().map_err(lease_error)?;
        Ok(Self {
            path,
            token: token.into(),
            file,
        })
    }
}

impl Drop for ServiceLease {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path)
            .ok()
            .and_then(|value| value.lines().next().map(ToOwned::to_owned))
            .as_deref()
            == Some(self.token.as_str())
        {
            let _ = fs::remove_file(&self.path);
        }
        let _ = FileExt::unlock(&self.file);
    }
}

fn lease_error(error: std::io::Error) -> ServiceError {
    ServiceError::new("DOWNLOAD_SERVICE_LEASE_FAILED", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{read_line, MAX_LINE_BYTES};
    use tokio::io::{duplex, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn rejects_an_oversized_line_before_the_peer_closes_the_connection() {
        let (mut writer, reader) = duplex(64 * 1024);
        let writer_task = tokio::spawn(async move {
            writer
                .write_all(&vec![b'a'; MAX_LINE_BYTES + 1])
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();

        let error = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_line(&mut reader, &mut line),
        )
        .await
        .expect("oversized input should be rejected without waiting for EOF")
        .unwrap_err();

        assert_eq!(error.code, "DOWNLOAD_SERVICE_REQUEST_TOO_LARGE");
        assert!(line.len() <= MAX_LINE_BYTES + 1);
        writer_task.abort();
    }
}
