#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

mod branch;
mod command;
mod diff;
mod ops;
mod repo;
mod status;

pub use branch::{branches, checkout_branch, create_branch};
pub use diff::{diff, file_diff};
pub use ops::{commit, discard, pull, push, stage, unstage};
pub use repo::repo_root;
pub use status::status;

