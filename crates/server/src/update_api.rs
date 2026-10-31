//! Updating the app from its GitHub releases (see `courseshelf_core::update`): check, install as a job, restart.

use crate::jobs::JobInfo;
use crate::{err500, App};
use anyhow::anyhow;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use courseshelf_core::update;
use std::sync::Arc;

type Shared = Arc<App>;

/// The version of this build. The workspace crates and app/src-tauri/tauri.conf.json carry the same number (a test checks it).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `{current, release}`: `release` is the newer published version (with notes and this system's installer), or null.
pub async fn api_check() -> Response {
    match tokio::task::spawn_blocking(|| update::check(VERSION)).await {
        Ok(Ok(release)) => Json(serde_json::json!({ "current": VERSION, "release": release })).into_response(),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response(),
        Err(e) => err500(anyhow!("{e}")),
    }
}

/// Downloads and installs the newer version as a job; the restart waits for `/api/update/restart`.
pub async fn api_install(State(app): State<Shared>) -> Response {
    let found = match tokio::task::spawn_blocking(|| update::check(VERSION)).await {
        Ok(Ok(Some(r))) => r,
        Ok(Ok(None)) => return (StatusCode::UNPROCESSABLE_ENTITY, format!("CourseShelf {VERSION} is already the newest version")).into_response(),
        Ok(Err(e)) => return (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response(),
        Err(e) => return err500(anyhow!("{e}")),
    };
    let Some(asset) = found.asset.clone() else {
        return (StatusCode::UNPROCESSABLE_ENTITY, format!("there is no ready-made installer for this system in {}; download it from {}", found.tag, found.page)).into_response();
    };
    let info = JobInfo { kind: "update".into(), course: "CourseShelf".into(), title: format!("CourseShelf {}", found.version), total: 1, ..Default::default() };
    let pending = app.restart.clone();
    let work: crate::jobs::Work = Box::new(move |ctx| {
        ctx.log(format!("downloading {} ({:.1} MB)", asset.name, asset.size as f64 / 1e6));
        let file = update::download(&asset, &update::work_dir())?;
        ctx.log("checked the download (size and SHA-256); installing");
        let restart = update::install(&file)?;
        ctx.log(match &restart {
            Some(update::Restart::App(_)) => "installed: restart CourseShelf to use the new version",
            Some(update::Restart::Installer(_)) => "downloaded: the installer starts when CourseShelf restarts",
            None => "opened the package in the system's installer: finish there, then restart CourseShelf",
        });
        *pending.lock().unwrap() = restart;
        ctx.update(|i| {
            i.done = 1;
            i.percent = 100.0;
        });
        Ok(())
    });
    match app.jobs.submit(info, work) {
        Ok(id) => Json(serde_json::json!({ "job": id })).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// Quits and starts the installed version. Your data is all on disk already (settings are saved as you change them).
pub async fn api_restart(State(app): State<Shared>) -> Response {
    let Some(restart) = app.restart.lock().unwrap().clone() else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "no update is waiting to start").into_response();
    };
    if let Err(e) = restart.run() {
        return err500(anyhow!("could not start the new version: {e}"));
    }
    // answer first, then quit
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::process::exit(0);
    });
    Json(serde_json::json!({ "restarting": true })).into_response()
}
