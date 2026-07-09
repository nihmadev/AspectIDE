pub mod client;
pub mod naming;
pub mod protocol;
pub mod tool_result;
pub mod types;

use std::collections::HashMap;
use std::sync::OnceLock;

use tokio::sync::Mutex as AsyncMutex;

use client::Connection;

pub(crate) fn registry() -> &'static AsyncMutex<HashMap<String, Connection>> {
    static REGISTRY: OnceLock<AsyncMutex<HashMap<String, Connection>>> = OnceLock::new();
    REGISTRY.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

pub use client::{all_status, call_tool, connect_server, disconnect_server, is_valid_id};
pub use naming::agent_tool_definitions;
pub use types::{
    McpServerConfig, McpServerStatus, McpToolInfo, MCP_SERVERS_KEY, MAX_LINE_BYTES,
    MAX_RESULT_CHARS, MAX_TOOL_NAME_LEN,
};
