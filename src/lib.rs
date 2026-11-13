mod client;
mod endpoint;
mod protocol;
mod server;
mod service;
mod service_cli;
mod standalone_service;
mod storage;
mod supervisor;

pub use client::{ServiceClient, ServiceClientError};
pub use endpoint::{read_endpoint, write_endpoint, Endpoint};
pub use protocol::{
    negotiate, ErrorEnvelope, HandshakeRequest, ServiceCapabilities, ServiceError,
    ServiceHandshake, ServiceHandshakeResponse, ServiceReply, ServiceRequest, APP_ID, DATA_API,
    HOST_API, SERVICE_API,
};
pub type ServiceEvent = DownloadEventPayload;
pub use server::ServiceServer;
pub use service::DownloadService;
pub use service_cli::{parse_service_args, ServiceRunOptions};
pub use standalone_service::StandaloneService;
pub use storage::{
    DownloadEventPayload, DownloadStatus, DownloadTask, MigrationOutcome, TaskStore,
};
pub use supervisor::{
    CommandServiceLauncher, ServiceLaunchSpec, ServiceLauncher, ServiceProcess, ServiceSupervisor,
    ServiceSupervisorError,
};
