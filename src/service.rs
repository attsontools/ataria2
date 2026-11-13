use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, OnceLock},
};

use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::{broadcast, oneshot, Mutex},
};

use crate::{DownloadEventPayload, DownloadStatus, ServiceError, TaskStore};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartDownloadRequest {
    url: String,
    download_dir: String,
    file_name: Option<String>,
    connections: Option<u8>,
    split: Option<u8>,
    min_split_size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartParams {
    request: StartDownloadRequest,
}

#[derive(Debug, Deserialize)]
struct TaskParams {
    id: String,
}

struct DownloadServiceInner {
    store: TaskStore,
    runtime_path: PathBuf,
    tasks: Mutex<HashMap<String, oneshot::Sender<()>>>,
    events: broadcast::Sender<DownloadEventPayload>,
}

#[derive(Clone)]
pub struct DownloadService {
    inner: Arc<DownloadServiceInner>,
}

impl DownloadService {
    pub fn open(data_directory: &Path, runtime_path: PathBuf) -> Result<Self, ServiceError> {
        let (store, _) = TaskStore::open(data_directory)?;
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            inner: Arc::new(DownloadServiceInner {
                store,
                runtime_path,
                tasks: Mutex::new(HashMap::new()),
                events,
            }),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEventPayload> {
        self.inner.events.subscribe()
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, ServiceError> {
        match method {
            "download.defaultDirectory" => {
                let path = dirs::download_dir()
                    .or_else(|| std::env::current_dir().ok())
                    .ok_or_else(|| {
                        ServiceError::new(
                            "DOWNLOAD_SERVICE_RUNTIME_UNAVAILABLE",
                            "cannot determine the default download directory",
                        )
                    })?;
                Ok(json!({ "path": path.to_string_lossy() }))
            }
            "download.tasks.list" => {
                let tasks = self.inner.store.list_tasks()?;
                Ok(json!({ "tasks": tasks }))
            }
            "download.tasks.start" => {
                let params: StartParams = serde_json::from_value(params).map_err(invalid_params)?;
                let id = self.start(params.request).await?;
                Ok(json!({ "id": id }))
            }
            "download.tasks.cancel" => {
                let params: TaskParams = serde_json::from_value(params).map_err(invalid_params)?;
                let sender = self
                    .inner
                    .tasks
                    .lock()
                    .await
                    .remove(&params.id)
                    .ok_or_else(|| {
                        ServiceError::new(
                            "DOWNLOAD_SERVICE_TASK_NOT_FOUND",
                            "running download task was not found",
                        )
                    })?;
                let _ = sender.send(());
                Ok(json!({ "cancelled": true }))
            }
            "download.tasks.clearCompleted" => {
                let count = self.inner.store.clear_completed()?;
                Ok(json!({ "count": count }))
            }
            "download.tasks.delete" => {
                let params: TaskParams = serde_json::from_value(params).map_err(invalid_params)?;
                self.inner.store.delete_task(&params.id)?;
                Ok(json!({ "deleted": true }))
            }
            "download.tasks.openFolder" => {
                let params: TaskParams = serde_json::from_value(params).map_err(invalid_params)?;
                let directory = self.inner.store.download_directory(&params.id)?;
                open_folder(&directory)?;
                Ok(json!({ "opened": true }))
            }
            _ => Err(ServiceError::new(
                "DOWNLOAD_SERVICE_METHOD_NOT_FOUND",
                format!("unknown download service method: {method}"),
            )),
        }
    }

    async fn start(&self, request: StartDownloadRequest) -> Result<String, ServiceError> {
        let url = request.url.trim().to_string();
        let download_dir = request.download_dir.trim().to_string();
        if url.is_empty() || download_dir.is_empty() {
            return Err(ServiceError::new(
                "DOWNLOAD_SERVICE_INVALID_PARAMS",
                "download URL and directory are required",
            ));
        }
        let file_name = request
            .file_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let id = self
            .inner
            .store
            .create_task(&url, &download_dir, file_name.as_deref())?;
        let mut args = vec![
            "--continue=true".to_string(),
            "--allow-overwrite=false".to_string(),
            "--auto-file-renaming=true".to_string(),
            format!("--dir={download_dir}"),
            format!(
                "--max-connection-per-server={}",
                request.connections.unwrap_or(16).clamp(1, 16)
            ),
            format!("--split={}", request.split.unwrap_or(16).clamp(1, 64)),
            format!(
                "--min-split-size={}",
                sanitize_min_split_size(request.min_split_size.as_deref())
            ),
            "--summary-interval=1".to_string(),
            "--show-console-readout=true".to_string(),
            "--console-log-level=notice".to_string(),
            "--download-result=hide".to_string(),
        ];
        if let Some(file_name) = &file_name {
            args.push(format!("--out={file_name}"));
        }
        args.push("--".to_string());
        args.push(url);

        let mut child = Command::new(&self.inner.runtime_path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                let failed = event(
                    &id,
                    DownloadStatus::Failed,
                    0.0,
                    None,
                    None,
                    Some(format!("aria2c 启动失败：{error}")),
                    None,
                );
                let _ = self.emit(failed);
                ServiceError::new(
                    "DOWNLOAD_SERVICE_SPAWN_FAILED",
                    format!("cannot start aria2 runtime: {error}"),
                )
            })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (cancel_sender, cancel_receiver) = oneshot::channel();
        self.inner
            .tasks
            .lock()
            .await
            .insert(id.clone(), cancel_sender);
        self.emit(event(
            &id,
            DownloadStatus::Queued,
            0.0,
            None,
            None,
            Some("aria2 任务已创建".into()),
            None,
        ))?;

        let service = self.clone();
        let task_id = id.clone();
        tokio::spawn(async move {
            let stdout_task = stdout.map(|stream| {
                let service = service.clone();
                let id = task_id.clone();
                tokio::spawn(async move { read_stdout(stream, service, id).await })
            });
            let stderr_task = stderr.map(|stream| {
                let service = service.clone();
                let id = task_id.clone();
                tokio::spawn(async move { read_stderr(stream, service, id).await })
            });
            let (terminal, cancelled) = tokio::select! {
                status = child.wait() => (match status {
                    Ok(status) if status.success() => event(
                        &task_id,
                        DownloadStatus::Completed,
                        100.0,
                        None,
                        None,
                        Some("下载完成".into()),
                        file_name.as_deref().map(|name| Path::new(&download_dir).join(name).to_string_lossy().into_owned()),
                    ),
                    Ok(status) => event(
                        &task_id,
                        DownloadStatus::Failed,
                        0.0,
                        None,
                        None,
                        Some(format!("aria2c 退出码：{status}")),
                        None,
                    ),
                    Err(error) => event(
                        &task_id,
                        DownloadStatus::Failed,
                        0.0,
                        None,
                        None,
                        Some(format!("等待 aria2c 结束失败：{error}")),
                        None,
                    ),
                }, false),
                _ = cancel_receiver => {
                    match child.kill().await {
                        Ok(()) => (event(
                            &task_id,
                            DownloadStatus::Cancelled,
                            0.0,
                            None,
                            None,
                            Some("已取消下载任务".into()),
                            None,
                        ), true),
                        Err(error) => (event(
                            &task_id,
                            DownloadStatus::Failed,
                            0.0,
                            None,
                            None,
                            Some(format!("取消 aria2c 失败：{error}")),
                            None,
                        ), false),
                    }
                }
            };
            if cancelled {
                if let Some(task) = &stdout_task {
                    task.abort();
                }
                if let Some(task) = &stderr_task {
                    task.abort();
                }
            } else {
                if let Some(task) = stdout_task {
                    let _ = task.await;
                }
                if let Some(task) = stderr_task {
                    let _ = task.await;
                }
            }
            service.inner.tasks.lock().await.remove(&task_id);
            let _ = service.emit(terminal);
        });
        Ok(id)
    }

    fn emit(&self, event: DownloadEventPayload) -> Result<(), ServiceError> {
        self.inner.store.persist_event(&event)?;
        let _ = self.inner.events.send(event);
        Ok(())
    }
}

async fn read_stdout<R>(stream: R, service: DownloadService, id: String)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\r', &mut buffer).await {
            Ok(0) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buffer);
                let parsed = parse_progress_line(&id, &line).or_else(|| {
                    parse_complete_line(&line).map(|path| {
                        event(
                            &id,
                            DownloadStatus::Running,
                            0.0,
                            None,
                            None,
                            None,
                            Some(path),
                        )
                    })
                });
                if let Some(event) = parsed {
                    let _ = service.emit(event);
                }
            }
            Err(error) => {
                let _ = service.emit(event(
                    &id,
                    DownloadStatus::Failed,
                    0.0,
                    None,
                    None,
                    Some(format!("读取 aria2 输出失败：{error}")),
                    None,
                ));
                break;
            }
        }
    }
}

async fn read_stderr<R>(stream: R, service: DownloadService, id: String)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while let Ok(bytes) = reader.read_line(&mut line).await {
        if bytes == 0 {
            break;
        }
        let message = line.trim();
        if !message.is_empty() {
            let _ = service.emit(event(
                &id,
                DownloadStatus::Running,
                0.0,
                None,
                None,
                Some(message.into()),
                None,
            ));
        }
        line.clear();
    }
}

fn event(
    id: &str,
    status: DownloadStatus,
    progress: f32,
    speed: Option<String>,
    eta: Option<String>,
    message: Option<String>,
    local_path: Option<String>,
) -> DownloadEventPayload {
    DownloadEventPayload {
        id: id.into(),
        status,
        progress,
        speed,
        eta,
        message,
        started_at: None,
        finished_at: None,
        local_path,
    }
}

fn parse_progress_line(id: &str, line: &str) -> Option<DownloadEventPayload> {
    static PROGRESS: OnceLock<Regex> = OnceLock::new();
    let progress = PROGRESS
        .get_or_init(|| Regex::new(r"\((\d+(?:\.\d+)?)%\)").expect("progress regex"))
        .captures(line)?
        .get(1)?
        .as_str()
        .parse::<f32>()
        .ok()?;
    Some(event(
        id,
        DownloadStatus::Running,
        progress,
        extract_token(line, "DL:"),
        extract_token(line, "ETA:"),
        None,
        None,
    ))
}

fn parse_complete_line(line: &str) -> Option<String> {
    static COMPLETE: OnceLock<Regex> = OnceLock::new();
    COMPLETE
        .get_or_init(|| {
            Regex::new(r"\[NOTICE\][^\n\r]*Download complete:\s*(.+?)\s*$").expect("complete regex")
        })
        .captures(line.trim_end_matches(['\r', '\n']))
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
}

fn extract_token(line: &str, prefix: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(prefix))
        .map(|value| value.trim_end_matches(']').to_string())
        .filter(|value| !value.is_empty())
}

fn sanitize_min_split_size(value: Option<&str>) -> String {
    static MIN_SPLIT: OnceLock<Regex> = OnceLock::new();
    let candidate = value.unwrap_or("1M").trim();
    if MIN_SPLIT
        .get_or_init(|| Regex::new(r"^\d+[KMG]?$").expect("min split regex"))
        .is_match(candidate)
    {
        candidate.into()
    } else {
        "1M".into()
    }
}

fn invalid_params(error: serde_json::Error) -> ServiceError {
    ServiceError::new("DOWNLOAD_SERVICE_INVALID_PARAMS", error.to_string())
}

fn open_folder(path: &Path) -> Result<(), ServiceError> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path).spawn().map_err(|error| {
        ServiceError::new(
            "DOWNLOAD_SERVICE_OPEN_FOLDER_FAILED",
            format!("cannot open download directory: {error}"),
        )
    })?;
    Ok(())
}
