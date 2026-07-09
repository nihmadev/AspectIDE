#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

mod io;
mod keybindings;
mod store;

pub use keybindings::default_keybinding_profile;
pub use store::SettingsStore;


