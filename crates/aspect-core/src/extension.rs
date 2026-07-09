use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub wasm_module: PathBuf,
    #[serde(default)]
    pub permissions: Vec<ExtensionHostPermission>,
    pub contributes: Vec<String>,
    #[serde(default)]
    pub commands: Vec<ExtensionCommandContribution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionStatus {
    Discovered,
    Active,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionContributionKind {
    Commands,
    Themes,
    Keybindings,
    Languages,
    Grammars,
    Snippets,
    Views,
    Menus,
    Settings,
    Debuggers,
    Tasks,
    ProblemMatchers,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionPoint {
    pub id: String,
    pub kind: ExtensionContributionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionCommandContribution {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub handler: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub permissions: Vec<ExtensionHostPermission>,
    pub contributes: Vec<String>,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub commands: Vec<ExtensionCommandContribution>,
    pub status: ExtensionStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionWasmPreflight {
    pub module_path: PathBuf,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionHostPermission {
    WorkspaceRead,
    WorkspaceWrite,
    NetworkAccess,
    ProcessSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionWasmImportKind {
    Function,
    Table,
    Memory,
    Global,
    Tag,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionWasmImport {
    pub module: String,
    pub name: String,
    pub kind: ExtensionWasmImportKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionWasmAbi {
    pub version: u32,
    pub entrypoint: String,
    pub required_exports: Vec<String>,
    pub optional_exports: Vec<String>,
    pub imports: Vec<ExtensionWasmImport>,
    pub exports_memory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionHostLimits {
    pub max_memory_pages: u32,
    #[ts(type = "number")]
    pub activation_timeout_ms: u64,
    #[ts(type = "number")]
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionHostActivationContract {
    pub abi: ExtensionWasmAbi,
    pub permissions: Vec<ExtensionHostPermission>,
    pub limits: ExtensionHostLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationCandidate {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub commands: Vec<ExtensionCommandContribution>,
    pub wasm_preflight: ExtensionWasmPreflight,
    pub host_contract: ExtensionHostActivationContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationBlocked {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub commands: Vec<ExtensionCommandContribution>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationPlan {
    pub candidates: Vec<ExtensionActivationCandidate>,
    pub blocked: Vec<ExtensionActivationBlocked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivated {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub wasm_module: PathBuf,
    #[ts(type = "number")]
    pub fuel_consumed: u64,
    #[ts(type = "number")]
    pub fuel_remaining: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationFailed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub wasm_module: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationReport {
    pub plan: ExtensionActivationPlan,
    pub activated: Vec<ExtensionActivated>,
    pub failed: Vec<ExtensionActivationFailed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionRegistration {
    pub extension_id: String,
    pub extension_name: String,
    pub extension_version: String,
    pub contribution: ExtensionContributionPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionUnavailable {
    pub extension_id: String,
    pub extension_name: String,
    pub extension_version: String,
    pub contribution: ExtensionContributionPoint,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionRegistry {
    pub activation: ExtensionActivationReport,
    pub registered: Vec<ExtensionContributionRegistration>,
    pub unavailable: Vec<ExtensionContributionUnavailable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionCommandRoute {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub handler: String,
    pub extension_id: String,
    pub extension_name: String,
    pub extension_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionCommandExecutionStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionCommandExecutionPhase {
    Routing,
    Activation,
    Handler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionCommandExecution {
    pub command_id: String,
    pub route: Option<ExtensionCommandRoute>,
    pub status: ExtensionCommandExecutionStatus,
    pub phase: ExtensionCommandExecutionPhase,
    pub reason: Option<String>,
    #[ts(type = "number")]
    pub duration_ms: u64,
    #[ts(type = "number | null")]
    pub activation_fuel_consumed: Option<u64>,
    #[ts(type = "number | null")]
    pub activation_fuel_remaining: Option<u64>,
    #[ts(type = "number | null")]
    pub handler_fuel_consumed: Option<u64>,
    #[ts(type = "number | null")]
    pub handler_fuel_remaining: Option<u64>,
}
