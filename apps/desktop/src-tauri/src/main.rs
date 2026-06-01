#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lux_desktop_lib::run();
}
