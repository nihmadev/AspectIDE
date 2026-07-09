use serde::Serialize;

/// Managed language runtimes the IDE can auto-provision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Node,
    Rust,
    Python,
    Go,
}

impl Runtime {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Node => "Node.js",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Go => "Go",
        }
    }

    /// The executable that, when resolvable, proves the runtime is installed.
    pub const fn marker_command(self) -> &'static str {
        match self {
            Self::Node => "npm",
            Self::Rust => "cargo",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "node" => Some(Self::Node),
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub const fn all() -> [Self; 4] {
        [Self::Node, Self::Rust, Self::Python, Self::Go]
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCatalogEntry {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub managed: bool,
    pub path: Option<String>,
    pub can_auto: bool,
    pub manual_hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RuntimeProvisionEvent {
    #[serde(rename_all = "camelCase")]
    Started { id: String, name: String },
    #[serde(rename_all = "camelCase")]
    Progress {
        id: String,
        percent: u8,
        step: String,
    },
    #[serde(rename_all = "camelCase")]
    Finished {
        id: String,
        success: bool,
        path: Option<String>,
        error: Option<String>,
    },
}
