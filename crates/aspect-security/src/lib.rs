#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

mod report;
mod splitter;
mod normalize;
mod launcher;
mod catastrophic;
mod rm_detect;
mod interpreter;
mod block_device;
mod risky;
mod read_only;

pub use report::{classify_shell_command, ShellSafetyReport};
