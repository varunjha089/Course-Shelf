//! Managing courses from the app: download missing lectures, point a course at an existing folder,
//! match hand-downloaded files to playlist positions, and move a course to another drive.

use crate::jobs::JobInfo;
use crate::{err500, App};
use anyhow::{anyhow, bail, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use courseshelf_core::{
    adopt,
    config::Config,
    fetcher::{self, Event},
    library, locations,
    progress::Progress,
    spec::{self, CourseRow},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

type Shared = Arc<App>;

pub(crate) fn bad(msg: impl Into<String>) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, msg.into()).into_response()
}

pub use courseshelf_core::locations::{locate, Located};

/// A course on disk, whether or not it has a row in courses.csv.
pub struct CourseRef {
    pub name: String,
    pub dir: PathBuf,
    pub state_dir: PathBuf,
    /// The drive courses.csv names for it; None for a folder found on a drive but not in the CSV.
    pub csv_drive: Option<String>,
}

pub(crate) fn resolve_course(app: &App, cfg: &Config, rows: &[CourseRow], name: &str, from_drive: Option<&str>) -> Result<CourseRef> {
    spec::check_course_name(name)?;
    if let Some(row) = rows.iter().find(|r| r.course == name) {
        let loc = locate(cfg, &app.home, row)?;
        return Ok(CourseRef { name: name.to_string(), dir: loc.dir, state_dir: loc.state_dir, csv_drive: Some(row.drive.clone()) });
    }
    for d in cfg.drives.iter().filter(|d| from_drive.map(|f| f == d.name).unwrap_or(true)) {
        if d.connected() {
            let dir = d.root.join(name);
            if dir.is_dir() && !library::scan_folder(&dir).is_empty() {
                return Ok(CourseRef { name: name.to_string(), dir, state_dir: d.state_dir(), csv_drive: None });
            }
        }
    }
    bail!("no course named '{name}' in courses.csv or on a connected drive")
}

pub(crate) fn load_all(app: &App) -> Result<(Config, Vec<CourseRow>)> {
    let cfg = Config::load(&app.home)?;
    let rows = spec::load_courses(&cfg.courses_csv)?;
    Ok((cfg, rows))
}

fn find_row<'a>(rows: &'a [CourseRow], course: &str) -> Result<&'a CourseRow> {
    rows.iter().find(|r| r.course == course).ok_or_else(|| anyhow!("'{course}' is not a row in courses.csv"))
}

fn est_bytes(cfg: &Config, row: &CourseRow, secs: f64) -> f64 {
    let factor = match row.quality_cap {
        None => 1.0,
        Some(c) if c >= 1080 => 1.0,
        Some(c) if c >= 720 => 0.6,
        Some(c) if c >= 480 => 0.35,
        Some(_) => 0.2,
    };
    secs / 3600.0 * cfg.ytdlp.est_gb_per_hour * 1e9 * factor
}

// ---------- overview ----------

#[derive(Serialize)]
pub(crate) struct ManageCourse {
    course: String,
    title: String,
    /// The drive the course's folder is on now ("folder" for a custom folder outside every drive).
    drive: String,
    /// The drive courses.csv names (where it would go with no custom folder).
    planned_drive: String,
    /// In courses.csv? Courses found on a drive without a row can still be moved.
    listed: bool,
    enabled: bool,
    playlist_url: String,
    items: String,
    note: String,
    drive_connected: bool,
    location: String,
    overridden: bool,
    folder_exists: bool,
    on_disk: usize,
    size_bytes: u64,
    free_bytes: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct DriveOut {
    name: String,
    path: String,
    connected: bool,
    free_bytes: Option<u64>,
    total_bytes: Option<u64>,
    budget_gb: Option<f64>,
    /// The folder sits on this Mac's own disk (not under /Volumes).
    is_mac: bool,
}

#[derive(Serialize)]
pub(crate) struct ManageOut {
    /// This build's version (the update check compares against it).
    version: &'static str,
    mac: Option<crate::MacInfo>,
    courses: Vec<ManageCourse>,
    drives: Vec<DriveOut>,
    ytdlp: Option<String>,
    /// The data folder (config.toml, courses.csv, annotations, books): shown so the user knows where their files are.
    home: String,
    error: Option<String>,
}

pub(crate) fn nice(course: &str) -> String {
    course.rsplit('/').next().unwrap_or(course).replace('_', " ")
}

pub async fn api_overview(State(app): State<Shared>) -> Json<ManageOut> {
    let r = tokio::task::spawn_blocking(move || -> ManageOut {
        let ytdlp = fetcher::tool_version_line("yt-dlp");
        let home = app.home.to_string_lossy().to_string();
        let (cfg, rows) = match load_all(&app) {
            Ok(x) => x,
            Err(e) => return ManageOut { version: crate::update_api::VERSION, mac: None, courses: vec![], drives: vec![], ytdlp, home, error: Some(format!("{e:#}")) },
        };
        let drives: Vec<DriveOut> = cfg
            .drives
            .iter()
            .map(|d| {
                let sp = if d.connected() { d.space() } else { None };
                DriveOut {
                    name: d.name.clone(),
                    path: d.root.to_string_lossy().to_string(),
                    connected: d.connected(),
                    free_bytes: sp.map(|x| x.0),
                    total_bytes: sp.map(|x| x.1),
                    budget_gb: d.budget_gb,
                    is_mac: !d.is_external(),
                }
            })
            .collect();
        let size_of = |dir: &Path| -> u64 { library::scan_folder(dir).iter().map(|l| l.size_bytes).sum() };
        let mut courses: Vec<ManageCourse> = rows
            .iter()
            .filter_map(|r| {
                let loc = locate(&cfg, &app.home, r).ok()?;
                let d = courseshelf_core::locations::host_drive(&cfg, r, &loc).ok()?;
                let lectures = library::scan_folder(&loc.dir);
                Some(ManageCourse {
                    course: r.course.clone(),
                    title: nice(&r.course),
                    drive: d.name.clone(),
                    planned_drive: r.drive.clone(),
                    listed: true,
                    enabled: r.enabled,
                    playlist_url: r.playlist_url.clone(),
                    items: r.items_text.clone(),
                    note: r.note.clone(),
                    drive_connected: if loc.overridden { loc.dir.exists() } else { d.connected() },
                    location: loc.dir.to_string_lossy().to_string(),
                    overridden: loc.overridden,
                    folder_exists: loc.dir.is_dir(),
                    on_disk: lectures.len(),
                    size_bytes: lectures.iter().map(|l| l.size_bytes).sum(),
                    free_bytes: if d.connected() { d.space().map(|s| s.0) } else { None },
                })
            })
            .collect();
        // folders found on a connected drive that courses.csv does not mention (a course's location inside the drive is
        // that course, already listed above)
        let locs = locations::load(&app.home);
        for d in cfg.drives.iter().filter(|d| d.connected()) {
            let located: Vec<String> = locs.values().filter_map(|p| locations::relative_inside(p, &d.root)).collect();
            for c in library::scan_root(&d.root) {
                if rows.iter().any(|r| r.course == c.course) || located.contains(&c.course) {
                    continue;
                }
                let dir = d.root.join(&c.course);
                courses.push(ManageCourse {
                    title: nice(&c.course),
                    drive: d.name.clone(),
                    planned_drive: d.name.clone(),
                    listed: false,
                    enabled: false,
                    playlist_url: String::new(),
                    items: String::new(),
                    note: String::new(),
                    drive_connected: true,
                    location: dir.to_string_lossy().to_string(),
                    overridden: false,
                    folder_exists: true,
                    on_disk: c.lectures.len(),
                    size_bytes: size_of(&dir),
                    free_bytes: d.space().map(|s| s.0),
                    course: c.course,
                });
            }
        }
        ManageOut { version: crate::update_api::VERSION, mac: crate::mac_info(&cfg), courses, drives, ytdlp, home, error: None }
    })
    .await;
    Json(r.unwrap_or(ManageOut { version: crate::update_api::VERSION, mac: None, courses: vec![], drives: vec![], ytdlp: None, home: String::new(), error: Some("internal error".into()) }))
}

// ---------- drives: change where a drive's folder is ----------

#[derive(Deserialize)]
pub struct DriveReq {
    name: String,
    path: String,
    #[serde(default)]
    create: bool,
}

/// Point a drive name (hdd, ssd, local) at another folder, e.g. after plugging in a disk with a different name.
/// Edits only that drive's `path` line in config.toml; comments and other settings stay.
pub async fn api_drive_set(State(app): State<Shared>, Json(r): Json<DriveReq>) -> Response {
    let res = (|| -> Result<serde_json::Value> {
        let cfg = Config::load(&app.home)?;
        cfg.drive(&r.name)?;
        let path = PathBuf::from(&r.path);
        if !path.is_absolute() {
            bail!("the folder must be an absolute path");
        }
        if !path.exists() {
            let parent = path.parent().ok_or_else(|| anyhow!("no parent folder"))?;
            if !r.create {
                bail!("{} does not exist (tick 'create it' to make the folder)", path.display());
            }
            if !parent.is_dir() {
                bail!("{} does not exist, so its sub-folder cannot be created (is the disk plugged in?)", parent.display());
            }
            std::fs::create_dir(&path)?;
        }
        if !path.is_dir() {
            bail!("{} is not a folder", path.display());
        }
        courseshelf_core::config::set_drive_path(&app.home, &r.name, &path.to_string_lossy())?;
        Ok(serde_json::json!({ "ok": true, "path": path.to_string_lossy() }))
    })();
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(format!("{e:#}")),
    }
}

// ---------- download ----------

#[derive(Deserialize)]
pub struct CourseReq {
    course: String,
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    force_titles: bool,
    /// Download into `<dest_root>/<course>` instead of the course's usual place: a drive's folder.
    #[serde(default)]
    dest_root: Option<String>,
    /// Download into exactly this folder (it may be new; its parent must exist). Wins over `dest_root`.
    #[serde(default)]
    dest_folder: Option<String>,
}

#[derive(Serialize)]
struct PlanOut {
    playlist_title: String,
    total: u32,
    wanted: usize,
    have: usize,
    missing: Vec<u32>,
    est_bytes: f64,
    free_bytes: Option<u64>,
    fits: bool,
    drive_connected: bool,
    title_warnings: Vec<String>,
    note: String,
    location: String,
}

/// Where a download goes when the user picked a destination: `<dest_root>/<course>`.
/// A configured drive's folder keeps that drive's state folder and budget; any other folder is a plain location.
fn dest_located(cfg: &Config, dest_root: &str, course: &str) -> Result<(Located, Option<courseshelf_core::config::Drive>)> {
    let root = PathBuf::from(dest_root);
    if !root.is_absolute() || !root.is_dir() {
        bail!("{dest_root} is not a folder that exists; connect the disk or pick another folder");
    }
    let root = root.canonicalize()?;
    let dir = root.join(course);
    if let Some(d) = cfg.drives.iter().find(|d| d.root.canonicalize().map(|r| r == root).unwrap_or(false)) {
        return Ok((Located { dir, state_dir: d.state_dir(), overridden: false }, Some(d.clone())));
    }
    let state_dir = dir.join(".courseshelf");
    Ok((Located { dir, state_dir, overridden: true }, None))
}

/// A course folder the user picked exactly. The parent must exist (a disk that is not plugged in is refused); the folder itself
/// may be new. Returns where it is, and the drive it lies on (or the folder itself when it is on no configured drive).
pub(crate) fn exact_located(cfg: &Config, row: &CourseRow, folder: &str) -> Result<(Located, courseshelf_core::config::Drive)> {
    let dir = PathBuf::from(folder);
    if !dir.is_absolute() {
        bail!("{folder} is not a full path");
    }
    let parent = dir.parent().filter(|p| p.is_dir()).ok_or_else(|| anyhow!("{folder} cannot be made here: its parent folder does not exist (is the disk plugged in?)"))?;
    let dir = parent.canonicalize()?.join(dir.file_name().ok_or_else(|| anyhow!("{folder} has no folder name"))?);
    let default = cfg.drive(&row.drive).ok().filter(|d| d.connected()).map(|d| d.root.join(&row.course));
    let at_default = default.as_ref().is_some_and(|d| d.canonicalize().unwrap_or_else(|_| d.clone()) == dir);
    let loc = if at_default {
        Located { dir: dir.clone(), state_dir: cfg.drive(&row.drive)?.state_dir(), overridden: false }
    } else {
        Located { state_dir: dir.join(".courseshelf"), dir: dir.clone(), overridden: true }
    };
    let host = courseshelf_core::locations::host_drive(cfg, row, &Located { overridden: true, ..loc.clone() })?;
    Ok((loc, host))
}

fn plan_for(app: &App, course: &str, refresh: bool, dest_root: Option<&str>, dest_folder: Option<&str>) -> Result<(PlanOut, Config, CourseRow, Located)> {
    let (cfg, rows) = load_all(app)?;
    let row = find_row(&rows, course)?.clone();
    if row.playlist_url.is_empty() {
        bail!("this course has no playlist_url in courses.csv yet");
    }
    let mut drive = cfg.drive(&row.drive)?.clone();
    let dest_folder = dest_folder.filter(|d| !d.is_empty());
    let dest_root = if dest_folder.is_some() { None } else { dest_root };
    let loc = match dest_root.filter(|d| !d.is_empty()) {
        _ if dest_folder.is_some() => {
            let (l, host) = exact_located(&cfg, &row, dest_folder.unwrap_or_default())?;
            drive = host;
            l
        }
        Some(d) => {
            let (l, dd) = dest_located(&cfg, d, &row.course)?;
            drive = dd.unwrap_or(courseshelf_core::config::Drive { name: "folder".into(), root: PathBuf::from(d), budget_gb: None });
            l
        }
        None => locate(&cfg, &app.home, &row)?,
    };
    let pl = fetcher::course_playlist(&app.home, &row, refresh)?;
    let total = pl.entries.len() as u32;
    let wanted = row.items.resolve(total);
    let connected = if dest_folder.is_some() {
        loc.dir.parent().is_some_and(Path::is_dir)
    } else if dest_root.is_some() {
        drive.root.is_dir()
    } else if loc.overridden {
        loc.dir.exists()
    } else {
        drive.connected()
    };
    let have_set = if loc.dir.is_dir() { library::downloaded_indexes(&loc.dir) } else { Default::default() };
    let missing: Vec<u32> = wanted.iter().copied().filter(|i| !have_set.contains(i)).collect();
    let secs: f64 = missing.iter().filter_map(|i| pl.entries.get(*i as usize - 1)).map(|e| e.duration.unwrap_or(0.0)).sum();
    let est = est_bytes(&cfg, &row, secs);
    let free = if connected { drive.space().map(|s| s.0).or_else(|| if loc.overridden && dest_root.is_none() && dest_folder.is_none() { Some(u64::MAX) } else { None }) } else { None };
    let budget = drive.budget_gb.map(|g| g * 1e9);
    let fits = connected && free.map(|f| est <= f as f64 * 0.97).unwrap_or(true) && budget.map(|b| est <= b).unwrap_or(true);
    let mut note = String::new();
    if let Some(m) = row.items.max_named() {
        if m > total {
            note = format!("items mention {m} but the playlist has only {total} videos");
        }
    }
    let title_warnings = fetcher::verify_titles(&app.home, &row, &pl);
    let out = PlanOut {
        playlist_title: pl.title,
        total,
        wanted: wanted.len(),
        have: wanted.len() - missing.len(),
        missing,
        est_bytes: est,
        free_bytes: free.filter(|f| *f != u64::MAX),
        fits,
        drive_connected: connected,
        title_warnings,
        note,
        location: loc.dir.to_string_lossy().to_string(),
    };
    Ok((out, cfg, row, loc))
}

pub async fn api_plan(State(app): State<Shared>, Json(r): Json<CourseReq>) -> Response {
    match tokio::task::spawn_blocking(move || plan_for(&app, &r.course, r.refresh, r.dest_root.as_deref(), r.dest_folder.as_deref()).map(|x| x.0)).await {
        Ok(Ok(p)) => Json(p).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

pub async fn api_download(State(app): State<Shared>, Json(r): Json<CourseReq>) -> Response {
    let app2 = app.clone();
    let dest = r.dest_folder.clone().filter(|d| !d.is_empty()).or_else(|| r.dest_root.clone().filter(|d| !d.is_empty()));
    let planned = tokio::task::spawn_blocking(move || plan_for(&app2, &r.course, false, r.dest_root.as_deref(), r.dest_folder.as_deref()).map(|x| (x, r.force_titles))).await;
    let ((plan, cfg, row, loc), force) = match planned {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return bad(format!("{e:#}")),
        Err(e) => return err500(anyhow!("{e}")),
    };
    if !plan.drive_connected {
        return bad(match &dest {
            Some(d) => format!("{d} is not available; connect the disk or pick another place to save"),
            None => format!("drive '{}' is not connected", row.drive),
        });
    }
    if plan.missing.is_empty() {
        return bad("nothing to download: every wanted lecture is already on the drive");
    }
    if !plan.fits {
        return bad("not enough room: the estimate is over the free space or the drive's budget in config.toml");
    }
    if !plan.title_warnings.is_empty() && !force {
        return bad(format!("the playlist's titles do not match the plan for {} lecture(s), e.g. {}. Check `items`, or confirm to download anyway.", plan.title_warnings.len(), plan.title_warnings[0]));
    }
    // the configured drive this download lands on (its state folder is where loc.state_dir points), if any
    let drive = cfg.drives.iter().find(|d| d.state_dir() == loc.state_dir).cloned();
    let chosen = dest.is_some();
    let home = app.home.clone();
    let default_dir = cfg.drive(&row.drive).map(|d| d.root.join(&row.course)).ok();
    let info = JobInfo {
        kind: "download".into(),
        course: row.course.clone(),
        drive: row.drive.clone(),
        title: row.course.rsplit('/').next().unwrap_or(&row.course).replace('_', " "),
        total: plan.missing.len() as u32,
        ..Default::default()
    };
    let missing = plan.missing.clone();
    let overridden = loc.overridden;
    let work: crate::jobs::Work = Box::new(move |ctx| {
        fetcher::require_tools()?;
        if let Some(d) = &drive {
            if !overridden {
                d.init()?;
            }
        }
        std::fs::create_dir_all(&loc.dir)?;
        if chosen {
            // remember where this course now lives, so the app finds it with no further step
            let dir = loc.dir.canonicalize()?;
            if default_dir.as_ref().map(|d| d.canonicalize().ok() == Some(dir.clone())).unwrap_or(false) {
                locations::clear(&home, &row.course)?;
            } else {
                locations::set(&home, &row.course, &dir)?;
            }
        }
        let mut last_logged = String::new();
        let mut sink = |ev: Event| match ev {
            Event::Line(l) => {
                if l != last_logged {
                    ctx.log(l.clone());
                    last_logged = l;
                }
            }
            Event::Item { index, title } => ctx.update(|i| {
                i.current_index = index;
                i.current_title = title;
                i.percent = 0.0;
            }),
            Event::Progress { index, percent, speed, eta } => ctx.update(|i| {
                i.current_index = index;
                i.percent = percent;
                i.speed = speed;
                i.eta = eta;
            }),
            Event::Checked { index, ok, message } => ctx.update(|i| {
                if ok {
                    i.done += 1;
                    if !message.is_empty() {
                        i.warnings.push(message.clone());
                    }
                } else {
                    i.failed.push(format!("{index:03}: {message}"));
                }
                i.percent = 0.0;
            }),
        };
        let res = fetcher::download_course(&cfg, &row, &missing, &loc.dir, &mut sink, &ctx.cancel)?;
        if !overridden && !chosen {
            let _ = fetcher::write_status(&cfg, &[res]);
        }
        Ok(())
    });
    match app.jobs.submit(info, work) {
        Ok(id) => Json(serde_json::json!({ "job": id })).into_response(),
        Err(e) => bad(e),
    }
}

pub async fn api_jobs(State(app): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "jobs": app.jobs.list() }))
}

#[derive(Deserialize)]
pub struct CancelReq {
    job: u64,
}

pub async fn api_cancel(State(app): State<Shared>, Json(r): Json<CancelReq>) -> Response {
    if app.jobs.cancel(r.job) {
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        bad("that job is not running or queued")
    }
}

// ---------- pointing a course at an existing folder ----------

#[derive(Deserialize)]
pub struct LocReq {
    course: String,
    path: Option<String>,
}

pub async fn api_location_set(State(app): State<Shared>, Json(r): Json<LocReq>) -> Response {
    let res = (|| -> Result<serde_json::Value> {
        let (_, rows) = load_all(&app)?;
        find_row(&rows, &r.course)?;
        let path = PathBuf::from(r.path.ok_or_else(|| anyhow!("no folder given"))?);
        let canon = path.canonicalize().map_err(|_| anyhow!("{} does not exist", path.display()))?;
        locations::set(&app.home, &r.course, &canon)?;
        Ok(serde_json::json!({ "location": canon.to_string_lossy(), "videos": library::scan_folder(&canon).len() }))
    })();
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(format!("{e:#}")),
    }
}

pub async fn api_location_clear(State(app): State<Shared>, Json(r): Json<LocReq>) -> Response {
    match locations::clear(&app.home, &r.course) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => bad(format!("{e:#}")),
    }
}

// ---------- adopt: number hand-downloaded files by matching titles ----------

pub async fn api_adopt_preview(State(app): State<Shared>, Json(r): Json<CourseReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (cfg, rows) = load_all(&app)?;
        let row = find_row(&rows, &r.course)?;
        if row.playlist_url.is_empty() {
            bail!("this course has no playlist_url in courses.csv yet");
        }
        let loc = locate(&cfg, &app.home, row)?;
        if !loc.dir.is_dir() {
            bail!("the course folder {} does not exist", loc.dir.display());
        }
        let pl = fetcher::course_playlist(&app.home, &row, r.refresh)?;
        let entries: Vec<(u32, String)> = pl.entries.iter().map(|e| (e.index, e.title.clone())).collect();
        let proposals = adopt::propose(&loc.dir, &entries);
        let unmatched_positions: Vec<u32> = entries.iter().map(|e| e.0).filter(|i| !proposals.iter().any(|p| p.index == Some(*i))).collect();
        Ok(serde_json::json!({ "playlist_title": pl.title, "playlist_total": entries.len(), "proposals": proposals, "positions_without_file": unmatched_positions.len() }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
pub struct AdoptApply {
    course: String,
    renames: Vec<(String, String)>,
}

pub async fn api_adopt_apply(State(app): State<Shared>, Json(r): Json<AdoptApply>) -> Response {
    let res = (|| -> Result<usize> {
        let (cfg, rows) = load_all(&app)?;
        let row = find_row(&rows, &r.course)?;
        let loc = locate(&cfg, &app.home, row)?;
        let stems = adopt::apply(&loc.dir, &r.renames)?;
        if let Ok(p) = Progress::open(&loc.state_dir) {
            for (old, new) in &stems {
                let _ = p.rename_key(&r.course, old, new);
            }
        }
        Ok(stems.len())
    })();
    match res {
        Ok(n) => Json(serde_json::json!({ "renamed": n })).into_response(),
        Err(e) => bad(format!("{e:#}")),
    }
}

// ---------- folder browser (for "point to an existing folder") ----------

#[derive(Deserialize)]
pub struct BrowseQ {
    path: Option<String>,
}

#[derive(Serialize)]
struct DirEntry {
    name: String,
    path: String,
    videos: usize,
}

/// Windows: the folder browser starts at the list of drives (C:\, D:\, ...), as a Mac starts at /Volumes.
fn drive_letters() -> serde_json::Value {
    let dirs: Vec<DirEntry> = (b'A'..=b'Z')
        .map(|c| format!("{}:\\", c as char))
        .filter(|d| Path::new(d).is_dir())
        .map(|d| DirEntry { name: d.clone(), path: d, videos: 0 })
        .collect();
    serde_json::json!({ "path": "", "parent": null, "videos_here": 0, "dirs": dirs, "home": courseshelf_core::config::home_dir() })
}

/// Windows' canonical paths start with \\?\ (C:\x becomes \\?\C:\x): show and hand out the usual form.
fn plain_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

pub async fn api_browse(State(_app): State<Shared>, Query(q): Query<BrowseQ>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start: PathBuf = match q.path.as_deref() {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ if cfg!(windows) => return Ok(drive_letters()),
            _ if cfg!(target_os = "macos") => PathBuf::from("/Volumes"),
            _ => ["/media", "/mnt", "/"].iter().map(PathBuf::from).find(|p| p.is_dir()).unwrap_or_default(),
        };
        let dir = plain_path(start.canonicalize().map_err(|_| anyhow!("{} does not exist", start.display()))?);
        if !dir.is_dir() {
            bail!("{} is not a folder", dir.display());
        }
        let mut dirs: Vec<DirEntry> = Vec::new();
        for e in std::fs::read_dir(&dir)?.flatten().take(500) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !e.path().is_dir() {
                continue;
            }
            let videos = std::fs::read_dir(e.path()).map(|rd| rd.flatten().filter(|f| matches!(f.path().extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(), Some("mp4" | "mkv" | "webm" | "m4v" | "mov"))).count()).unwrap_or(0);
            dirs.push(DirEntry { path: e.path().to_string_lossy().to_string(), name, videos });
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let here = library::scan_folder(&dir).len();
        Ok(serde_json::json!({
            "path": dir.to_string_lossy(),
            // Windows: above a drive's root comes the list of drives (an empty path)
            "parent": dir.parent().map(|p| p.to_string_lossy().to_string()).or_else(|| cfg!(windows).then(String::new)),
            "videos_here": here,
            "dirs": dirs,
            "home": courseshelf_core::config::home_dir().map(|h| h.to_string_lossy().to_string()),
        }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}


// ---------- tidy: shrink and hide the side files ----------

#[derive(Deserialize)]
pub struct TidyReq {
    /// One course; empty for every course folder that can be reached now.
    #[serde(default)]
    course: String,
}

pub async fn api_tidy(State(app): State<Shared>, Json(r): Json<TidyReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (cfg, rows) = load_all(&app)?;
        let folders = if r.course.is_empty() {
            courseshelf_core::tidy::course_folders(&cfg, &app.home, None)
        } else {
            vec![resolve_course(&app, &cfg, &rows, &r.course, None)?.dir]
        };
        let mut total = courseshelf_core::tidy::Report::default();
        for dir in folders.iter().filter(|d| d.is_dir()) {
            total.add(&courseshelf_core::tidy::tidy(dir)?);
        }
        Ok(serde_json::json!({ "folders": folders.len(), "shrunk": total.shrunk, "bytes_saved": total.bytes_saved, "hidden": total.hidden }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}
