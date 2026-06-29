#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]

//! agent-browser bridge: drives the `agent-browser` CLI for AI browser
//! automation. Split into focused submodules:
//! - [`types`]: request/response DTOs and internal carriers
//! - [`resolver`]: trusted binary resolution + tuning constants
//! - [`validate`]: domain/proxy/provider validation + sanitisation
//! - [`version`]: version string parsing/comparison
//! - [`process`]: subprocess execution + bounded output parsing
//! - [`operations`]: status/invoke/stream/dashboard/skills/install/read-image
//!
//! The Tauri command wrappers stay here so their `crate::agent_browser::*`
//! paths and the public API are unchanged.

mod operations;
mod process;
mod resolver;
mod types;
mod validate;
mod version;

pub use operations::{dashboard, install, invoke, read_image, skills, status, stream_status};
pub use types::{
    AgentBrowserDashboardRequest, AgentBrowserDashboardResponse, AgentBrowserInstallRequest,
    AgentBrowserInstallResponse, AgentBrowserInvokeRequest, AgentBrowserInvokeResponse,
    AgentBrowserReadImageRequest, AgentBrowserReadImageResponse, AgentBrowserSkillsRequest,
    AgentBrowserSkillsResponse, AgentBrowserStatusRequest, AgentBrowserStatusResponse,
    AgentBrowserStreamStatusRequest, AgentBrowserStreamStatusResponse,
};

// ── Tauri Commands ──

#[tauri::command]
pub async fn agent_browser_status(
    request: Option<AgentBrowserStatusRequest>,
) -> Result<AgentBrowserStatusResponse, String> {
    status(request.unwrap_or_default()).await
}

#[tauri::command]
pub async fn agent_browser_invoke(
    request: AgentBrowserInvokeRequest,
) -> Result<AgentBrowserInvokeResponse, String> {
    invoke(request).await
}

#[tauri::command]
pub async fn agent_browser_install(
    request: Option<AgentBrowserInstallRequest>,
) -> Result<AgentBrowserInstallResponse, String> {
    install(request.unwrap_or_default()).await
}

#[tauri::command]
pub async fn agent_browser_read_image(
    request: AgentBrowserReadImageRequest,
) -> Result<AgentBrowserReadImageResponse, String> {
    read_image(request).await
}

#[tauri::command]
pub async fn agent_browser_stream_status(
    request: AgentBrowserStreamStatusRequest,
) -> Result<AgentBrowserStreamStatusResponse, String> {
    stream_status(request).await
}

#[tauri::command]
pub async fn agent_browser_dashboard(
    request: AgentBrowserDashboardRequest,
) -> Result<AgentBrowserDashboardResponse, String> {
    dashboard(request).await
}

#[tauri::command]
pub async fn agent_browser_skills(
    request: Option<AgentBrowserSkillsRequest>,
) -> Result<AgentBrowserSkillsResponse, String> {
    skills(request.unwrap_or_default()).await
}
