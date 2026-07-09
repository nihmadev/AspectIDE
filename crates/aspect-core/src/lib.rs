#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

mod concurrency;
pub use concurrency::{
    acquire_scan_workers, resolve_scan_threads, scan_threads, set_scan_concurrency,
    ScanConcurrency, ScanWorkers,
};

mod file_view;
pub use file_view::*;

mod error;
pub use error::*;

mod workspace;
pub use workspace::*;

mod fs;
pub use fs::*;

mod search;
pub use search::*;

mod terminal;
pub use terminal::*;

mod git;
pub use git::*;

mod settings;
pub use settings::*;

mod extension;
pub use extension::*;

mod debug;
pub use debug::*;

mod lsp;
pub use lsp::*;

mod events;
pub use events::*;
