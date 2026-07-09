#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(
    clippy::items_after_statements,
    clippy::large_stack_frames,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::significant_drop_tightening,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

mod platform;
pub use platform::*;

mod crypto;
pub use crypto::*;

mod error;
pub use error::*;

mod archive;
pub use archive::*;

mod fs;
pub use fs::*;

mod command;
pub use command::*;

pub mod resolve;
pub use resolve::*;

mod io;
pub use io::*;

pub mod runtime;
pub mod lsp;
