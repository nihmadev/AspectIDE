#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

//! Debug Adapter Protocol (DAP) client for `AspectIDE`.
//!
//! The crate is split into three layers:
//! - [`protocol`]: transport-agnostic wire types, frame encoding/decoding,
//!   request builders, response parsers, and launch-variable resolution.
//! - [`session`]: process/TCP transport plus the `DebugSessionManager` state
//!   machine that drives a live adapter connection.
//! - [`workspace`]: built-in adapter discovery and `.vscode/launch.json` parsing.
//!
//! The public API is re-exported here so consumers depend on `aspect_dap::*`
//! without knowing the internal module layout.

mod protocol;
mod session;
mod workspace;

pub use protocol::{
    attach_request, configuration_done_request, disconnect_request, drain_dap_frames,
    encode_dap_message, evaluate_context_name, evaluate_request, execution_action_command,
    execution_request, initialize_request, launch_request, parse_breakpoints_response,
    parse_dap_message, parse_evaluate_response, parse_scopes_response, parse_stack_trace_response,
    parse_threads_response, parse_variables_response, scopes_request, set_breakpoints_request,
    stack_trace_request, threads_request, variables_request, DapEvent, DapFrame, DapMessage,
    DapRequest, DapResponse,
};
pub use session::{DebugSessionManager, DebugSessionUpdate};
pub use workspace::{
    adapter_matches_configuration, workspace_debug_adapter_for_configuration,
    workspace_debug_adapters, workspace_debug_info,
};
