// CourseShelf.app: a native window around the local viewer server (crates/server).
// The server listens on 127.0.0.1 only, on a random port, and the window is the only
// thing that holds its per-run key.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{WebviewUrl, WebviewWindowBuilder};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let home = courseshelf_core::config::default_home();
            // First start on a computer: make the data folder with a starter config.toml and an empty courses.csv (existing files are never changed).
            courseshelf_core::config::ensure_home(&home)?;
            let run = courseshelf_server::start_background(home, 0, None)?;
            // Test hook: when COURSESHELF_URL_FILE is set, write the window's local address there so a script can talk to
            // the running app's server. Not set in normal use.
            if let Some(f) = std::env::var_os("COURSESHELF_URL_FILE") {
                let _ = std::fs::write(f, run.url());
            }
            let url: tauri::Url = run.url().parse()?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("CourseShelf")
                .inner_size(1360.0, 900.0)
                .min_inner_size(900.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("CourseShelf failed to start");
}
