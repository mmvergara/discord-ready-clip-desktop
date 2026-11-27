// Release builds on Windows are GUI apps: no console window behind the main window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    discord_ready_clip_lib::run()
}
