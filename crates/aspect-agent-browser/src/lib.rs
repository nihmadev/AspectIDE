#![allow(clippy::module_name_repetitions)]

mod types;
mod resolver;
mod validate;
mod version;
mod process;
mod probes;
mod status;
mod invoke;
mod read_image;
mod stream;
mod install;

pub use types::{
    AgentBrowserDashboardRequest, AgentBrowserDashboardResponse, AgentBrowserInstallRequest,
    AgentBrowserInstallResponse, AgentBrowserInstallStep, AgentBrowserInvokeRequest,
    AgentBrowserInvokeResponse, AgentBrowserReadImageRequest, AgentBrowserReadImageResponse,
    AgentBrowserSkillsRequest, AgentBrowserSkillsResponse, AgentBrowserStatusRequest,
    AgentBrowserStatusResponse, AgentBrowserStreamStatusRequest, AgentBrowserStreamStatusResponse,
};
pub use resolver::{
    binary_source_label, resolve_binary, resolve_binary_with_source, set_app_data_dir,
    set_desktop_dir, BinarySource, DEFAULT_MAX_OUTPUT, DEFAULT_TIMEOUT_SECS, INSTALL_TIMEOUT_SECS,
    MAX_IMAGE_BYTES, MAX_OUTPUT_CAP, MAX_TIMEOUT_SECS,
};
pub use status::status;
pub use invoke::invoke;
pub use read_image::read_image;
pub use stream::{dashboard, skills, stream_status};
pub use install::install;
