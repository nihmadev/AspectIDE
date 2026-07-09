//! Tauri command wrappers for agent-browser operations.
//! All actual logic lives in the `aspect-agent-browser` crate.

#![allow(clippy::module_name_repetitions)]

pub use aspect_agent_browser::{
    dashboard, install, invoke, read_image, skills, status, stream_status,
    AgentBrowserDashboardRequest, AgentBrowserDashboardResponse, AgentBrowserInstallRequest,
    AgentBrowserInstallResponse, AgentBrowserInvokeRequest, AgentBrowserInvokeResponse,
    AgentBrowserReadImageRequest, AgentBrowserReadImageResponse, AgentBrowserSkillsRequest,
    AgentBrowserSkillsResponse, AgentBrowserStatusRequest, AgentBrowserStatusResponse,
    AgentBrowserStreamStatusRequest, AgentBrowserStreamStatusResponse,
};

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
    app: tauri::AppHandle,
    request: Option<AgentBrowserInstallRequest>,
) -> Result<AgentBrowserInstallResponse, String> {
    install(&app, request.unwrap_or_default()).await
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
