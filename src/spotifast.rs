//! Spotifast X desktop command.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod entrypoint;

fn main() -> eframe::Result<()> {
    entrypoint::run()
}
