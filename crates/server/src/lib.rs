//! CourseShelf local server: JSON API + video streaming + the embedded web UI.
//! Binds 127.0.0.1 only; every /api and /media request needs the per-run token cookie.

use anyhow::{anyhow, Result};
use axum::{
    body::Body,
    extract::{Path as AxPath, Query, State},
    http::{header, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
mod annot;
mod collections;
mod jobs;
mod manage;

use courseshelf_core::{addpapers, config::Config, library, locations, papers, progress::Progress, spec};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tower::ServiceExt;
use tower_http::services::ServeFile;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Ui;

#[derive(Clone)]
pub struct App {
    pub home: PathBuf,
    pub token: String,
    pub port: u16,
    /// Serve the UI from this folder instead of the embedded copy (for development).
    pub ui_dir: Option<PathBuf>,
    papers_cache: Arc<std::sync::Mutex<Option<PapersCache>>>,
    /// One download at a time, at least 3 s apart (arXiv asks for polite use).
    dl_gate: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
    pub(crate) jobs: Arc<jobs::Jobs>,
    /// Drive folders already tidied in this run (each is tidied once, the first time it is seen connected).
    tidied: Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
}

struct PapersCache {
    mtime: std::time::SystemTime,
    papers: Arc<Vec<papers::Paper>>,
}

type Shared = Arc<App>;

// ---------- JSON shapes ----------

#[derive(Serialize)]
struct DriveInfo {
    name: String,
    path: String,
    connected: bool,
    free_bytes: Option<u64>,
    total_bytes: Option<u64>,
    budget_gb: Option<f64>,
}

#[derive(Serialize)]
struct LectureInfo {
    #[serde(flatten)]
    lecture: library::Lecture,
    position: f64,
    done: bool,
    last_watched: i64,
}

#[derive(Serialize)]
struct CourseInfo {
    drive: String,
    course: String,
    title: String,
    /// In courses.csv?
    listed: bool,
    enabled: bool,
    priority: Option<u32>,
    note: String,
    playlist_url: String,
    items: String,
    connected: bool,
    speed: f64,
    done_count: usize,
    /// Folder the lectures are read from.
    location: String,
    /// True when locations.json points the course somewhere other than <drive>/<course>.
    overridden: bool,
    /// Every place this course lives (hdd, this Mac, ...). `drive`/`location` above describe the copy being played.
    copies: Vec<CopyInfo>,
    lectures: Vec<LectureInfo>,
}

#[derive(Serialize, Clone)]
pub(crate) struct CopyInfo {
    drive: String,
    location: String,
    connected: bool,
    is_mac: bool,
    overridden: bool,
    lectures: usize,
    size_bytes: u64,
}

/// How much of this Mac the courses may use (config.toml `[mac]`), for the Copy to Mac button.
#[derive(Serialize)]
pub(crate) struct MacInfo {
    drive: String,
    path: String,
    used_bytes: u64,
    max_bytes: Option<u64>,
    free_bytes: Option<u64>,
}

pub(crate) fn mac_info(cfg: &Config) -> Option<MacInfo> {
    let d = cfg.mac_drive()?;
    let used = if d.connected() { library::scan_root(&d.root).iter().flat_map(|c| c.lectures.iter()).map(|l| l.size_bytes).sum() } else { 0 };
    Some(MacInfo { drive: d.name.clone(), path: d.root.to_string_lossy().to_string(), used_bytes: used, max_bytes: cfg.mac_max_bytes(), free_bytes: if d.connected() { d.space().map(|s| s.0) } else { None } })
}

#[derive(Serialize)]
struct Recent {
    drive: String,
    course: String,
    course_title: String,
    key: String,
    title: String,
    file: String,
    position: f64,
    duration: f64,
    last_watched: i64,
}

#[derive(Serialize)]
struct StateOut {
    drives: Vec<DriveInfo>,
    courses: Vec<CourseInfo>,
    recent: Vec<Recent>,
    mac: Option<MacInfo>,
    error: Option<String>,
}

fn nice_title(course: &str) -> String {
    course.rsplit('/').next().unwrap_or(course).replace('_', " ")
}

/// One place a course lives on disk.
struct CopyRef {
    drive: String,
    dir: PathBuf,
    state_dir: PathBuf,
    connected: bool,
    overridden: bool,
    is_mac: bool,
    lectures: Vec<library::Lecture>,
    /// For a location inside a drive: that drive's progress database and the folder's name there. Progress saved while the
    /// folder was also listed under that name (before the scan skipped it) is still read from there.
    alias: Option<(PathBuf, String)>,
}

fn open_progress(state_dir: &Path, connected: bool) -> Option<Progress> {
    if !connected || !(state_dir.exists() || state_dir.parent().map(|p| p.exists()).unwrap_or(false)) {
        return None;
    }
    Progress::open(state_dir).ok()
}

fn build_state(home: &Path) -> StateOut {
    let mut out = StateOut { drives: vec![], courses: vec![], recent: vec![], mac: None, error: None };
    let cfg = match Config::load(home) {
        Ok(c) => c,
        Err(e) => {
            out.error = Some(format!("{e:#}"));
            return out;
        }
    };
    let rows = match spec::load_courses(&cfg.courses_csv) {
        Ok(r) => r,
        Err(e) => {
            out.error = Some(format!("{e:#}"));
            vec![]
        }
    };
    let overrides = locations::load(home);
    let mac_name = cfg.mac_drive().map(|d| d.name.clone());
    // the drive each location lies inside (if any) and its folder name there
    let inside: std::collections::HashMap<&String, (&courseshelf_core::config::Drive, String)> = overrides
        .iter()
        .filter_map(|(course, p)| {
            cfg.drives.iter().filter(|d| d.connected()).find_map(|d| locations::relative_inside(p, &d.root).map(|rel| (course, (d, rel))))
        })
        .collect();

    // 1. find every copy of every course, drive by drive
    let mut order: Vec<String> = vec![];
    let mut copies: std::collections::HashMap<String, Vec<CopyRef>> = Default::default();
    for d in &cfg.drives {
        let space = if d.connected() { d.space() } else { None };
        out.drives.push(DriveInfo {
            name: d.name.clone(),
            path: d.root.to_string_lossy().to_string(),
            connected: d.connected(),
            free_bytes: space.map(|s| s.0),
            total_bytes: space.map(|s| s.1),
            budget_gb: d.budget_gb,
        });
        let is_mac = mac_name.as_deref() == Some(d.name.as_str());
        let on_disk = if d.connected() { library::scan_root(&d.root) } else { vec![] };
        for r in rows.iter().filter(|r| r.drive == d.name) {
            if !order.contains(&r.course) {
                order.push(r.course.clone());
            }
            if let Some(p) = overrides.get(&r.course) {
                // a location inside a drive (e.g. on this Mac) is a copy on that drive
                let host = inside.get(&r.course);
                copies.entry(r.course.clone()).or_default().push(CopyRef {
                    drive: host.map_or_else(|| d.name.clone(), |(h, _)| h.name.clone()),
                    dir: p.clone(),
                    state_dir: p.join(".courseshelf"),
                    connected: p.is_dir(),
                    overridden: true,
                    is_mac: host.is_some_and(|(h, _)| mac_name.as_deref() == Some(h.name.as_str())),
                    lectures: library::scan_folder(p),
                    alias: host.map(|(h, rel)| (h.state_dir(), rel.clone())),
                });
            } else if !on_disk.iter().any(|c| c.course == r.course) {
                // a row always gets an entry, so an unplugged drive still shows its course
                copies.entry(r.course.clone()).or_default().push(CopyRef { drive: d.name.clone(), dir: d.root.join(&r.course), state_dir: d.state_dir(), connected: d.connected(), overridden: false, is_mac, lectures: vec![], alias: None });
            }
        }
        for c in on_disk {
            // a course listed under another name through its location: already counted above
            if overrides.contains_key(&c.course) || inside.values().any(|(h, rel)| h.name == d.name && *rel == c.course) {
                continue;
            }
            if !order.contains(&c.course) {
                order.push(c.course.clone());
            }
            let dir = d.root.join(&c.course);
            copies.entry(c.course.clone()).or_default().push(CopyRef { drive: d.name.clone(), dir, state_dir: d.state_dir(), connected: true, overridden: false, is_mac, lectures: c.lectures, alias: None });
        }
    }
    out.mac = mac_info(&cfg);

    // 2. one entry per course, played from the Mac copy when there is one
    for name in order {
        let mut cs = copies.remove(&name).unwrap_or_default();
        let mut seen_dirs = std::collections::HashSet::new();
        cs.retain(|c| seen_dirs.insert(c.dir.canonicalize().unwrap_or_else(|_| c.dir.clone())));
        if cs.is_empty() {
            continue;
        }
        let row = rows.iter().find(|r| r.course == name);
        let active_idx = cs
            .iter()
            .position(|c| c.is_mac && c.connected && !c.lectures.is_empty())
            .or_else(|| cs.iter().position(|c| c.connected && !c.lectures.is_empty()))
            .unwrap_or(0);
        // watch progress: the newest record from any connected copy wins
        let mut merged: std::collections::HashMap<String, courseshelf_core::progress::Prog> = Default::default();
        let mut speed = 1.0;
        let mut merge = |records: std::collections::HashMap<String, courseshelf_core::progress::Prog>| {
            for (k, v) in records {
                match merged.get(&k) {
                    Some(old) if old.last_watched > v.last_watched => {}
                    _ => {
                        merged.insert(k, v);
                    }
                }
            }
        };
        for (i, c) in cs.iter().enumerate() {
            let Some(p) = open_progress(&c.state_dir, c.connected) else { continue };
            if i == active_idx {
                speed = p.speed(&name);
            }
            merge(p.all(&name).unwrap_or_default());
            if let Some((old, alias)) = c.alias.as_ref().and_then(|(state_dir, alias)| Some((open_progress(state_dir, true)?, alias))) {
                merge(old.all(alias).unwrap_or_default());
            }
        }
        let active = &cs[active_idx];
        let lectures: Vec<LectureInfo> = active
            .lectures
            .iter()
            .cloned()
            .map(|l| {
                let p = merged.get(&l.key).cloned().unwrap_or_default();
                LectureInfo { position: p.position, done: p.done, last_watched: p.last_watched, lecture: l }
            })
            .collect();
        for l in &lectures {
            if l.last_watched > 0 && l.position > 5.0 && !l.done {
                let dur = merged.get(&l.lecture.key).map(|p| p.duration).filter(|d| *d > 0.0).or(l.lecture.duration).unwrap_or(0.0);
                out.recent.push(Recent {
                    drive: active.drive.clone(),
                    course: name.clone(),
                    course_title: nice_title(&name),
                    key: l.lecture.key.clone(),
                    title: l.lecture.title.clone(),
                    file: l.lecture.file.clone(),
                    position: l.position,
                    duration: dur,
                    last_watched: l.last_watched,
                });
            }
        }
        let copy_infos: Vec<CopyInfo> = cs
            .iter()
            .map(|c| CopyInfo {
                drive: c.drive.clone(),
                location: c.dir.to_string_lossy().to_string(),
                connected: c.connected,
                is_mac: c.is_mac,
                overridden: c.overridden,
                lectures: c.lectures.len(),
                size_bytes: c.lectures.iter().map(|l| l.size_bytes).sum(),
            })
            .collect();
        out.courses.push(CourseInfo {
            drive: active.drive.clone(),
            course: name.clone(),
            title: nice_title(&name),
            listed: row.is_some(),
            enabled: row.map(|r| r.enabled).unwrap_or(true),
            priority: row.map(|r| r.priority),
            note: row.map(|r| r.note.clone()).unwrap_or_default(),
            playlist_url: row.map(|r| r.playlist_url.clone()).unwrap_or_default(),
            items: row.map(|r| r.items_text.clone()).unwrap_or_default(),
            connected: active.connected,
            speed,
            done_count: lectures.iter().filter(|l| l.done).count(),
            location: active.dir.to_string_lossy().to_string(),
            overridden: active.overridden,
            copies: copy_infos,
            lectures,
        });
    }
    out.recent.sort_by(|a, b| b.last_watched.cmp(&a.last_watched));
    out.recent.truncate(12);
    out
}

// ---------- paths ----------

/// Join `rel` onto `root`, refusing anything that could leave it.
fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    let mut out = root.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(s) if !s.to_string_lossy().starts_with('.') => out.push(s),
            _ => return None,
        }
    }
    let canon = out.canonicalize().ok()?;
    let root_canon = root.canonicalize().ok()?;
    canon.starts_with(&root_canon).then_some(canon)
}

fn drive_root(app: &App, drive: &str) -> Option<(Config, PathBuf)> {
    let cfg = Config::load(&app.home).ok()?;
    let d = cfg.drive(drive).ok()?;
    if !d.connected() {
        return None;
    }
    let root = d.root.clone();
    Some((cfg, root))
}

// ---------- security ----------

fn host_ok(app: &App, headers: &HeaderMap) -> bool {
    let h = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    h == format!("127.0.0.1:{}", app.port) || h == format!("localhost:{}", app.port)
}

fn cookie_ok(app: &App, headers: &HeaderMap) -> bool {
    let want = format!("cs={}", app.token);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|c| c.split(';').any(|p| p.trim() == want))
}

async fn guard(State(app): State<Shared>, req: Request<Body>, next: Next) -> Response {
    if !host_ok(&app, req.headers()) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    let p = req.uri().path();
    if (p.starts_with("/api/") || p.starts_with("/media/") || p.starts_with("/pdf/")) && !cookie_ok(&app, req.headers()) {
        return (StatusCode::UNAUTHORIZED, "open CourseShelf from the app window").into_response();
    }
    next.run(req).await
}

// ---------- handlers ----------

/// Tidies (see `courseshelf_core::tidy`) the course folders of every drive that is connected and not yet tidied in this run,
/// in the background: courses downloaded before tidying existed, or on a disk just plugged in.
fn tidy_new_drives(app: &App) {
    let Ok(cfg) = Config::load(&app.home) else { return };
    let fresh: Vec<PathBuf> = {
        let mut seen = app.tidied.lock().unwrap();
        cfg.drives.iter().filter(|d| d.connected() && d.root.is_dir()).map(|d| d.root.clone()).filter(|r| seen.insert(r.clone())).collect()
    };
    if fresh.is_empty() {
        return;
    }
    let home = app.home.clone();
    std::thread::spawn(move || {
        for root in fresh {
            for dir in courseshelf_core::tidy::course_folders(&cfg, &home, Some(&root)) {
                let _ = courseshelf_core::tidy::tidy(&dir);
            }
        }
    });
}

async fn api_state(State(app): State<Shared>) -> Json<StateOut> {
    tidy_new_drives(&app);
    let home = app.home.clone();
    Json(tokio::task::spawn_blocking(move || build_state(&home)).await.unwrap_or_else(|e| StateOut {
        drives: vec![],
        courses: vec![],
        recent: vec![],
        mac: None,
        error: Some(format!("internal error: {e}")),
    }))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Pos {
    drive: String,
    course: String,
    key: String,
    position: f64,
    duration: f64,
}

/// Every connected copy's progress database for a course. Writes go to all of them, so the Mac copy and the
/// HDD copy never disagree about what you have watched.
fn progress_all(app: &App, course: &str) -> Result<Vec<Progress>> {
    spec::check_course_name(course)?;
    let cfg = Config::load(&app.home)?;
    let mut out: Vec<Progress> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut add = |dir: &Path, state: PathBuf, out: &mut Vec<Progress>| -> Result<()> {
        if dir.is_dir() && !seen.contains(&state) {
            seen.push(state.clone());
            out.push(Progress::open(&state)?);
        }
        Ok(())
    };
    if let Some(p) = locations::load(&app.home).get(course) {
        add(p, p.join(".courseshelf"), &mut out)?;
    }
    for d in cfg.drives.iter().filter(|d| d.connected()) {
        add(&d.root.join(course), d.state_dir(), &mut out)?;
    }
    if out.is_empty() {
        return Err(anyhow!("no connected copy of this course"));
    }
    Ok(out)
}

/// A file inside a course (rel = "<course>/<file>"): an explicit location wins over the drive root.
fn resolve_file(app: &App, drive: &str, rel: &str) -> Option<PathBuf> {
    for (course, path) in &locations::load(&app.home) {
        if let Some(rest) = rel.strip_prefix(&format!("{course}/")) {
            return safe_join(path, rest);
        }
    }
    let (_, root) = drive_root(app, drive)?;
    safe_join(&root, rel)
}

pub(crate) fn err500(e: anyhow::Error) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response()
}

async fn api_progress(State(app): State<Shared>, Json(p): Json<Pos>) -> Response {
    match progress_all(&app, &p.course).and_then(|dbs| {
        let mut done = false;
        for db in &dbs {
            done |= db.save(&p.course, &p.key, p.position, p.duration)?;
        }
        Ok(done)
    }) {
        Ok(done) => Json(serde_json::json!({ "done": done })).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DoneReq {
    drive: String,
    course: String,
    key: String,
    done: bool,
}

async fn api_done(State(app): State<Shared>, Json(p): Json<DoneReq>) -> Response {
    match progress_all(&app, &p.course).and_then(|dbs| dbs.iter().try_for_each(|db| db.set_done(&p.course, &p.key, p.done))) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SpeedReq {
    drive: String,
    course: String,
    speed: f64,
}

async fn api_speed(State(app): State<Shared>, Json(p): Json<SpeedReq>) -> Response {
    match progress_all(&app, &p.course).and_then(|dbs| dbs.iter().try_for_each(|db| db.set_speed(&p.course, p.speed))) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
struct SubQ {
    drive: String,
    course: String,
    file: String,
}

async fn api_subs(State(app): State<Shared>, Query(q): Query<SubQ>) -> Response {
    if !q.file.ends_with(".vtt") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(p) = resolve_file(&app, &q.drive, &format!("{}/{}", q.course, q.file)) else { return StatusCode::NOT_FOUND.into_response() };
    match std::fs::read_to_string(p) {
        Ok(t) => ([(header::CONTENT_TYPE, "text/vtt; charset=utf-8")], t).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn media(State(app): State<Shared>, AxPath((drive, rel)): AxPath<(String, String)>, req: Request<Body>) -> Response {
    let ext = Path::new(&rel).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["mp4", "m4v", "mkv", "webm", "mov"].contains(&ext.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(path) = resolve_file(&app, &drive, &rel) else { return StatusCode::NOT_FOUND.into_response() };
    match ServeFile::new(path).oneshot(req).await {
        Ok(r) => r.map(Body::new).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// ---------- papers (read-only view of the paper list, `[papers] list`) ----------

#[derive(Serialize)]
struct PaperOut {
    #[serde(flatten)]
    paper: papers::Paper,
    /// The row names a local PDF and that file exists now.
    held: bool,
    /// The row names a local PDF but the file is not there (old folder move).
    missing_local: bool,
    banks: Vec<papers::Bank>,
    /// arXiv or a direct .pdf link: the app can fetch it.
    downloadable: bool,
    /// Already downloaded into the inbox (not yet part of the library).
    inbox: Option<papers::InboxRec>,
}

#[derive(Serialize)]
struct PapersOut {
    available: bool,
    error: Option<String>,
    library: String,
    /// The paper list file, and whether it is the starter list in the data folder (no `[papers]` section).
    list: String,
    starter: bool,
    papers: Vec<PaperOut>,
}

fn load_papers_cached(app: &App, db: &Path) -> Result<Arc<Vec<papers::Paper>>> {
    let mtime = std::fs::metadata(db)?.modified()?;
    let mut g = app.papers_cache.lock().map_err(|_| anyhow!("papers cache poisoned"))?;
    if let Some(c) = g.as_ref() {
        if c.mtime == mtime {
            return Ok(c.papers.clone());
        }
    }
    let list = Arc::new(papers::load(db)?);
    *g = Some(PapersCache { mtime, papers: list.clone() });
    Ok(list)
}

/// The `[papers]` library, or (no such section) the starter list in the data folder, so "Add papers" works on a fresh install.
fn papers_paths(app: &App) -> Result<courseshelf_core::config::PapersPaths> {
    Ok(Config::load(&app.home)?.papers.unwrap_or_else(|| courseshelf_core::config::PapersPaths::starter(&app.home)))
}

/// The paper list; a starter list that "Add papers" has not made yet is an empty list (a configured list that is missing is an error).
pub(crate) fn load_list(app: &App, pp: &courseshelf_core::config::PapersPaths) -> Result<Arc<Vec<papers::Paper>>> {
    if pp.starter && !pp.list_csv.exists() {
        return Ok(Arc::new(vec![]));
    }
    load_papers_cached(app, &pp.list_csv)
}

async fn api_papers(State(app): State<Shared>) -> Json<PapersOut> {
    let r = tokio::task::spawn_blocking(move || -> Result<PapersOut> {
        let pp = papers_paths(&app)?;
        let list = load_list(&app, &pp)?;
        let banks = papers::load_banks(&pp.banks_csv);
        let inbox: std::collections::HashMap<String, papers::InboxRec> =
            papers::load_inbox(&pp.inbox).into_iter().filter(|r| pp.inbox.join(&r.file).is_file()).map(|r| (r.id.clone(), r)).collect();
        let out = list
            .iter()
            .map(|p| {
                let held = p.local_path.as_ref().map(|x| x.is_file()).unwrap_or(false);
                PaperOut {
                    held,
                    missing_local: p.local_path.is_some() && !held,
                    banks: banks.get(&p.link).cloned().unwrap_or_default(),
                    downloadable: is_downloadable(p),
                    inbox: inbox.get(&p.id).cloned(),
                    paper: p.clone(),
                }
            })
            .collect();
        Ok(PapersOut {
            available: true,
            error: None,
            library: pp.library.to_string_lossy().to_string(),
            list: pp.list_csv.to_string_lossy().to_string(),
            starter: pp.starter,
            papers: out,
        })
    })
    .await;
    Json(match r {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => PapersOut { available: false, error: Some(format!("{e:#}")), library: String::new(), list: String::new(), starter: false, papers: vec![] },
        Err(e) => PapersOut { available: false, error: Some(format!("internal error: {e}")), library: String::new(), list: String::new(), starter: false, papers: vec![] },
    })
}

// ---------- adding papers to the list (the only write to it; see core::addpapers) ----------

#[derive(Deserialize)]
struct LookupReq {
    text: String,
}

#[derive(Serialize)]
struct Existing {
    id: String,
    sno: u32,
    title: String,
}

#[derive(Serialize)]
struct Looked {
    paper: addpapers::NewPaper,
    /// Already in the paper list (matched on arXiv id, DOI or link): it cannot be added again.
    existing: Option<Existing>,
}

#[derive(Serialize)]
struct LookupOut {
    found: Vec<Looked>,
    not_understood: Vec<String>,
    errors: Vec<String>,
    /// Topics already used in the list, in the list's order (to choose from).
    topics: Vec<String>,
    /// How the papers will be written: the library's add_command, or None (appended to the list).
    add_command: Option<String>,
}

fn existing_in(list: &[papers::Paper], p: &addpapers::NewPaper) -> Option<Existing> {
    let doi = p.doi.as_ref().map(|d| d.to_ascii_lowercase());
    list.iter()
        .find(|x| {
            (p.arxiv.is_some() && x.arxiv == p.arxiv) || x.link == p.link || doi.as_ref().map(|d| x.link.to_ascii_lowercase().ends_with(d.as_str())).unwrap_or(false)
        })
        .map(|x| Existing { id: x.id.clone(), sno: x.sno, title: x.title.clone() })
}

async fn api_papers_lookup(State(app): State<Shared>, Json(r): Json<LookupReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> Result<LookupOut> {
        let pp = papers_paths(&app)?;
        let list = load_list(&app, &pp)?;
        let (refs, not_understood) = addpapers::parse_refs(&r.text);
        if refs.is_empty() {
            return Err(anyhow!("no arXiv id, arXiv link or DOI found in the text"));
        }
        let (found, errors) = addpapers::lookup(&refs);
        let mut topics: Vec<String> = vec![];
        for p in list.iter() {
            if !p.topic.is_empty() && !topics.contains(&p.topic) {
                topics.push(p.topic.clone());
            }
        }
        let found = found.into_iter().map(|p| Looked { existing: existing_in(&list, &p), paper: p }).collect();
        let add_command = (!pp.add_command.is_empty()).then(|| pp.add_command.join(" "));
        Ok(LookupOut { found, not_understood, errors, topics, add_command })
    })
    .await;
    match res {
        Ok(Ok(o)) => Json(o).into_response(),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response(),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
struct AddPapersReq {
    papers: Vec<addpapers::NewPaper>,
}

/// Write the chosen papers into the list, as a job: through `[papers] add_command` when set, otherwise appended with a backup.
async fn api_papers_add(State(app): State<Shared>, Json(r): Json<AddPapersReq>) -> Response {
    let bad = |m: String| (StatusCode::UNPROCESSABLE_ENTITY, m).into_response();
    if r.papers.is_empty() {
        return bad("choose at least one paper".into());
    }
    if let Some(p) = r.papers.iter().find(|p| p.title.trim().is_empty() || p.link.trim().is_empty()) {
        return bad(format!("a paper has no title or link ({})", p.link));
    }
    let pp = match papers_paths(&app) {
        Ok(x) => x,
        Err(e) => return bad(format!("{e:#}")),
    };
    if let Ok(list) = load_list(&app, &pp) {
        if let Some((p, e)) = r.papers.iter().find_map(|p| existing_in(&list, p).map(|e| (p, e))) {
            return bad(format!("“{}” is already in the list (#{} {})", p.title, e.sno, e.title));
        }
    }
    let n = r.papers.len() as u32;
    let info = jobs::JobInfo { kind: "addpapers".into(), course: "papers:add".into(), title: format!("Add {n} paper{}", if n == 1 { "" } else { "s" }), total: n, ..Default::default() };
    let a = app.clone();
    let papers_in = r.papers;
    let work: jobs::Work = Box::new(move |ctx| {
        let home = a.home.clone();
        if pp.add_command.is_empty() {
            if pp.starter && addpapers::start_list(&pp.list_csv)? {
                ctx.log(format!("started a new paper list: {}", pp.list_csv.display()));
            }
            let backup = addpapers::append_to_list(&pp.list_csv, &papers_in, &home.join("papers").join("backups"))?;
            ctx.log(format!("appended {} row(s) to {}; backup: {}", papers_in.len(), pp.list_csv.display(), backup.display()));
        } else {
            let dir = home.join("cache");
            std::fs::create_dir_all(&dir)?;
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let json = dir.join(format!("add_papers_{stamp}.json"));
            std::fs::write(&json, serde_json::to_vec_pretty(&papers_in)?)?;
            ctx.log(format!("running: {} {}", pp.add_command.join(" "), json.display()));
            let out = courseshelf_core::tools::command(&pp.add_command[0])
                .args(&pp.add_command[1..])
                .arg(&json)
                .current_dir(&pp.library)
                .output()
                .map_err(|e| anyhow!("could not run {}: {e}", pp.add_command[0]))?;
            for l in String::from_utf8_lossy(&out.stdout).lines().chain(String::from_utf8_lossy(&out.stderr).lines()) {
                ctx.log(l.to_string());
            }
            if !out.status.success() {
                let last: Vec<String> = String::from_utf8_lossy(&out.stderr).lines().chain(String::from_utf8_lossy(&out.stdout).lines()).filter(|l| !l.trim().is_empty()).map(String::from).collect();
                return Err(anyhow!("the library's add command stopped: {}", last.last().cloned().unwrap_or_else(|| format!("exit code {:?}", out.status.code()))));
            }
            let _ = std::fs::remove_file(&json);
        }
        ctx.update(|i| i.done = i.total);
        Ok(())
    });
    match app.jobs.submit(info, work) {
        Ok(j) => Json(serde_json::json!({ "job": j })).into_response(),
        Err(e) => bad(e),
    }
}

fn is_downloadable(p: &papers::Paper) -> bool {
    p.arxiv.is_some() || ((p.link.starts_with("https://") || p.link.starts_with("http://")) && p.link.to_ascii_lowercase().ends_with(".pdf"))
}

#[derive(Deserialize)]
struct DlReq {
    id: String,
}

fn pdf_pages(path: &Path) -> u32 {
    courseshelf_core::tools::command("pdfinfo")
        .arg(path)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().find_map(|l| l.strip_prefix("Pages:").and_then(|n| n.trim().parse().ok())))
        .unwrap_or(0)
}

/// Fetch one paper into the inbox with curl. Only arXiv and direct .pdf links; the file must verify as a PDF.
fn download_blocking(app: &App, id: &str) -> Result<papers::InboxRec> {
    let pp = papers_paths(app)?;
    let list = load_list(app, &pp)?;
    let p = list.iter().find(|p| p.id == id).ok_or_else(|| anyhow!("unknown paper"))?;
    if p.local_path.as_ref().map(|x| x.is_file()).unwrap_or(false) {
        return Err(anyhow!("this paper is already in the library"));
    }
    if !is_downloadable(p) {
        return Err(anyhow!("only arXiv and direct PDF links can be fetched here; open the source link in your browser"));
    }
    if let Some(r) = papers::load_inbox(&pp.inbox).into_iter().find(|r| r.id == id && pp.inbox.join(&r.file).is_file()) {
        return Ok(r);
    }
    let url = match &p.arxiv {
        Some(a) => format!("https://arxiv.org/pdf/{a}"),
        None => p.link.clone(),
    };
    std::fs::create_dir_all(&pp.inbox)?;
    let tmp = pp.inbox.join(format!(".part-{id}"));
    let _ = std::fs::remove_file(&tmp);
    let out = courseshelf_core::tools::command("curl")
        .args(["-sSL", "--fail", "--max-filesize", "209715200", "--max-time", "240", "-A", "CourseShelf/0.1 (personal research library)", "-o"])
        .arg(&tmp)
        .arg(&url)
        .output()
        .map_err(|e| anyhow!("could not run curl: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let bytes = std::fs::metadata(&tmp)?.len();
    let (head, tail) = {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&tmp)?;
        let mut head = vec![0u8; 512.min(bytes as usize)];
        f.read_exact(&mut head)?;
        let n = 2048.min(bytes as usize);
        f.seek(SeekFrom::End(-(n as i64)))?;
        let mut tail = vec![0u8; n];
        f.read_exact(&mut tail)?;
        (head, tail)
    };
    if let Err(why) = papers::check_pdf(&head, &tail, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!("not saved: {why}"));
    }
    let name = papers::inbox_name(p.arxiv.as_deref(), &p.link, &p.title);
    std::fs::rename(&tmp, pp.inbox.join(&name))?;
    let rec = papers::InboxRec {
        id: id.to_string(),
        link: p.link.clone(),
        file: name.clone(),
        bytes,
        pages: pdf_pages(&pp.inbox.join(&name)),
        when_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    papers::append_inbox(&pp.inbox, &rec)?;
    Ok(rec)
}

async fn api_download(State(app): State<Shared>, Json(r): Json<DlReq>) -> Response {
    // One at a time, and never faster than one request per 3 s.
    let mut gate = app.dl_gate.lock().await;
    if let Some(last) = *gate {
        let wait = std::time::Duration::from_secs(3).saturating_sub(last.elapsed());
        tokio::time::sleep(wait).await;
    }
    let a = app.clone();
    let res = tokio::task::spawn_blocking(move || download_blocking(&a, &r.id)).await;
    *gate = Some(std::time::Instant::now());
    match res {
        Ok(Ok(rec)) => Json(serde_json::json!({ "ok": true, "file": rec.file, "bytes": rec.bytes, "pages": rec.pages })).into_response(),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response(),
        Err(e) => err500(anyhow!("internal error: {e}")),
    }
}

async fn pdf_inbox(State(app): State<Shared>, AxPath(id): AxPath<String>, req: Request<Body>) -> Response {
    let Ok(pp) = papers_paths(&app) else { return StatusCode::NOT_FOUND.into_response() };
    let Some(rec) = papers::load_inbox(&pp.inbox).into_iter().find(|r| r.id == id) else { return StatusCode::NOT_FOUND.into_response() };
    match safe_join(&pp.inbox, &rec.file) {
        Some(p) => serve_pdf(p, req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The local PDF of a paper, only if it really sits inside the paper library (symlinks into Teaching/ resolve there).
fn paper_file(app: &App, id: &str) -> Option<PathBuf> {
    let pp = papers_paths(app).ok()?;
    let list = load_list(app, &pp).ok()?;
    let p = list.iter().find(|p| p.id == id)?;
    let canon = p.local_path.as_ref()?.canonicalize().ok()?;
    let root = pp.library.canonicalize().ok()?;
    (canon.starts_with(&root) && canon.extension().map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false)).then_some(canon)
}

async fn serve_pdf(path: PathBuf, req: Request<Body>) -> Response {
    match ServeFile::new(path).oneshot(req).await {
        Ok(r) => {
            let mut r = r.map(Body::new).into_response();
            r.headers_mut().insert(header::CONTENT_DISPOSITION, "inline".parse().unwrap());
            r
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn pdf_paper(State(app): State<Shared>, AxPath(id): AxPath<String>, req: Request<Body>) -> Response {
    match paper_file(&app, &id) {
        Some(p) => serve_pdf(p, req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pdf_bank(State(app): State<Shared>, AxPath((folder, pdf)): AxPath<(String, String)>, req: Request<Body>) -> Response {
    let Ok(pp) = papers_paths(&app) else { return StatusCode::NOT_FOUND.into_response() };
    if !pdf.to_ascii_lowercase().ends_with(".pdf") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match safe_join(&pp.banks_dir, &format!("{folder}/{pdf}")) {
        Some(p) => serve_pdf(p, req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct OpenReq {
    /// "paper" | "bank" | "external"
    kind: String,
    id: Option<String>,
    folder: Option<String>,
    pdf: Option<String>,
    url: Option<String>,
    /// open in the default app (false) or reveal in Finder (true)
    reveal: Option<bool>,
}

/// Hand a file or link to macOS (`open`): the default PDF viewer, Finder, or the default browser.
async fn api_open(State(app): State<Shared>, Json(r): Json<OpenReq>) -> Response {
    let reveal = r.reveal.unwrap_or(false);
    let target: Option<String> = match r.kind.as_str() {
        "external" => r.url.filter(|u| u.starts_with("https://") || u.starts_with("http://")),
        "paper" => r.id.and_then(|id| paper_file(&app, &id)).map(|p| p.to_string_lossy().to_string()),
        "inbox" => papers_paths(&app).ok().and_then(|pp| {
            let id = r.id?;
            let rec = papers::load_inbox(&pp.inbox).into_iter().find(|x| x.id == id)?;
            safe_join(&pp.inbox, &rec.file).map(|x| x.to_string_lossy().to_string())
        }),
        "bank" => papers_paths(&app).ok().and_then(|pp| {
            let (f, p) = (r.folder?, r.pdf?);
            safe_join(&pp.banks_dir, &format!("{f}/{p}")).map(|x| x.to_string_lossy().to_string())
        }),
        "book" => r.id.and_then(|id| annot::book_file(&app, &id)).map(|p| p.to_string_lossy().to_string()),
        "collection" => r.id.and_then(|id| collections::collection_file(&app, &id)).map(|p| p.to_string_lossy().to_string()),
        // a saved single video: reveal the downloaded file (or its folder); only paths recorded in singles.json are opened
        "single" => r.id.and_then(|id| {
            let it = courseshelf_core::singles::load(&app.home).items.into_iter().find(|x| x.id == id)?;
            it.files.first().cloned().filter(|f| std::path::Path::new(f).exists()).or(it.dest)
        }),
        _ => None,
    };
    let Some(t) = target else { return StatusCode::NOT_FOUND.into_response() };
    match courseshelf_core::tools::open(std::ffi::OsStr::new(&t), reveal && r.kind != "external") {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err500(e.into()),
    }
}

fn settings_path(app: &App) -> PathBuf {
    app.home.join("ui-settings.json")
}

fn read_settings(app: &App) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(settings_path(app))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

async fn api_settings_get(State(app): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::Value::Object(read_settings(&app)))
}

/// Merge the posted keys into ui-settings.json. Only the keys the UI uses are accepted.
async fn api_settings_post(State(app): State<Shared>, Json(patch): Json<serde_json::Map<String, serde_json::Value>>) -> Response {
    let mut cur = read_settings(&app);
    for (k, v) in patch {
        let ok = match k.as_str() {
            "font" => v.as_f64().map(|n| (12.0..=40.0).contains(&n)).unwrap_or(false),
            "theme" => matches!(v.as_str(), Some("auto" | "light" | "dark")),
            "autonext" => v.is_boolean(),
            _ => false,
        };
        if ok {
            cur.insert(k, v);
        }
    }
    let path = settings_path(&app);
    let tmp = path.with_extension("json.tmp");
    let res = serde_json::to_string_pretty(&serde_json::Value::Object(cur.clone()))
        .map_err(anyhow::Error::from)
        .and_then(|t| std::fs::write(&tmp, t).map_err(anyhow::Error::from))
        .and_then(|_| std::fs::rename(&tmp, &path).map_err(anyhow::Error::from));
    match res {
        Ok(()) => Json(serde_json::Value::Object(cur)).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
struct IndexQ {
    t: Option<String>,
}

async fn static_files(State(app): State<Shared>, uri: axum::http::Uri, Query(q): Query<IndexQ>) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let mut resp = match load_asset(&app, path).or_else(|| load_asset(&app, "index.html")) {
        Some((bytes, mime)) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    if q.t.as_deref() == Some(app.token.as_str()) {
        resp.headers_mut().insert(
            header::SET_COOKIE,
            format!("cs={}; Path=/; HttpOnly; SameSite=Strict", app.token).parse().unwrap(),
        );
    }
    resp.headers_mut().insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    resp
}

fn load_asset(app: &App, path: &str) -> Option<(Vec<u8>, String)> {
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    if let Some(dir) = &app.ui_dir {
        let p = safe_join(dir, path)?;
        return std::fs::read(p).ok().map(|b| (b, mime));
    }
    Ui::get(path).map(|f| (f.data.into_owned(), mime))
}

pub fn router(app: Shared) -> Router {
    Router::new()
        .route("/api/state", get(api_state))
        .route("/api/progress", post(api_progress))
        .route("/api/done", post(api_done))
        .route("/api/speed", post(api_speed))
        .route("/api/subs", get(api_subs))
        .route("/api/settings", get(api_settings_get).post(api_settings_post))
        .route("/api/papers", get(api_papers))
        .route("/api/open", post(api_open))
        .route("/api/papers/download", post(api_download))
        .route("/api/papers/lookup", post(api_papers_lookup))
        .route("/api/papers/add", post(api_papers_add))
        .route("/api/manage", get(manage::api_overview))
        .route("/api/manage/plan", post(manage::api_plan))
        .route("/api/manage/download", post(manage::api_download))
        .route("/api/manage/jobs", get(manage::api_jobs))
        .route("/api/manage/cancel", post(manage::api_cancel))
        .route("/api/manage/location", post(manage::api_location_set))
        .route("/api/manage/location/clear", post(manage::api_location_clear))
        .route("/api/manage/adopt/preview", post(manage::api_adopt_preview))
        .route("/api/manage/adopt/apply", post(manage::api_adopt_apply))
        .route("/api/manage/move/plan", post(manage::api_move_plan))
        .route("/api/manage/move/start", post(manage::api_move_start))
        .route("/api/manage/browse", get(manage::api_browse))
        .route("/api/singles", get(manage::api_singles))
        .route("/api/singles/add", post(manage::api_single_add))
        .route("/api/singles/remove", post(manage::api_single_remove))
        .route("/api/singles/download", post(manage::api_single_download))
        .route("/api/manage/drive", post(manage::api_drive_set))
        .route("/api/manage/mac/copy", post(manage::api_mac_copy))
        .route("/api/manage/mac/remove", post(manage::api_mac_remove))
        .route("/api/manage/tidy", post(manage::api_tidy))
        .route("/pdf/paper/{id}", get(pdf_paper))
        .route("/pdf/inbox/{id}", get(pdf_inbox))
        .route("/pdf/bank/{folder}/{pdf}", get(pdf_bank))
        .route("/media/{drive}/{*rel}", get(media))
        .merge(annot::routes())
        .merge(collections::routes())
        .fallback(static_files)
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

pub struct Running {
    pub port: u16,
    pub token: String,
}

impl Running {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }
}

/// Start the server on its own thread and return once it is listening. `port` 0 = any free port.
/// Progress saved under a folder's own name (while a course's location inside a drive was also listed as an extra course) is
/// copied once into the course's own database, newest record winning, so each course keeps its progress in one place.
/// Nothing is deleted; `build_state` still reads the old records too. Returns the number of lectures brought over.
fn merge_alias_progress(home: &Path) -> usize {
    let Ok(cfg) = Config::load(home) else { return 0 };
    let mut n = 0;
    for (course, dir) in locations::load(home) {
        let Some((drive, alias)) = cfg.drives.iter().filter(|d| d.connected()).find_map(|d| locations::relative_inside(&dir, &d.root).map(|rel| (d, rel))) else {
            continue;
        };
        let (Some(old), true) = (open_progress(&drive.state_dir(), true), dir.is_dir()) else { continue };
        let rows = old.export(&alias).unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        if let Ok(own) = Progress::open(&dir.join(".courseshelf")) {
            let have = own.all(&course).unwrap_or_default();
            let newer: Vec<_> = rows.into_iter().filter(|(k, p)| have.get(k).is_none_or(|h| p.last_watched > h.last_watched)).collect();
            if !newer.is_empty() {
                n += own.import(&course, &newer, None).unwrap_or(0);
            }
        }
    }
    n
}

pub fn start_background(home: PathBuf, port: u16, ui_dir: Option<PathBuf>) -> Result<Running> {
    let token = courseshelf_core::random_hex(16)?;
    {
        let home = home.clone();
        std::thread::spawn(move || merge_alias_progress(&home));
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<u16>>();
    let tok = token.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(e.into()));
                return;
            }
        };
        rt.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await {
                Ok(l) => l,
                Err(e) => {
                    let _ = tx.send(Err(e.into()));
                    return;
                }
            };
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
            let app = Arc::new(App { home, token: tok, port, ui_dir, papers_cache: Default::default(), dl_gate: Default::default(), jobs: jobs::Jobs::new(), tidied: Default::default() });
            let _ = tx.send(Ok(port));
            let _ = axum::serve(listener, router(app)).await;
        });
    });
    let port = rx.recv()??;
    Ok(Running { port, token })
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A course whose location (locations.json) lies inside another drive's folder, under a different name: it is listed
    /// once, as a copy on that drive, and progress saved under the folder's own name (the old duplicate entry) still counts.
    #[test]
    fn located_course_inside_a_drive_is_listed_once() {
        let base = std::env::temp_dir().join(format!("cs_state_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (home, root) = (base.join("home"), base.join("mac"));
        let dir = root.join("Extra").join("Provider").join("Course");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("001 - Lecture 1.mp4"), b"video").unwrap();
        std::fs::write(
            home.join("config.toml"),
            format!(
                "courses_csv = \"courses.csv\"\n[mac]\ndrive = \"local\"\n[drives.local]\npath = {:?}\n[drives.ssd]\npath = {:?}\n",
                root.to_string_lossy(),
                base.join("unplugged").to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::write(
            home.join("courses.csv"),
            "course,playlist_url,items,priority,drive,quality_cap,enabled,note\nProvider/Course,,all,1,ssd,,no,\n",
        )
        .unwrap();
        locations::set(&home, "Provider/Course", &dir).unwrap();
        // progress written while the folder was also listed as "Extra/Provider/Course" on the Mac drive
        let old = Progress::open(&root.join(".courseshelf")).unwrap();
        old.save("Extra/Provider/Course", "001 - Lecture 1", 886.0, 4700.0).unwrap();
        drop(old);

        let state = build_state(&home);
        assert!(state.error.is_none(), "{:?}", state.error);
        let found: Vec<&CourseInfo> = state.courses.iter().filter(|c| c.course.ends_with("Course")).collect();
        assert_eq!(found.len(), 1, "listed once: {:?}", found.iter().map(|c| &c.course).collect::<Vec<_>>());
        let c = found[0];
        assert_eq!(c.course, "Provider/Course");
        assert_eq!(c.drive, "local");
        assert!(c.copies[0].is_mac);
        assert_eq!(c.lectures[0].position, 886.0);

        // the old record is copied once into the course's own database (and the drive's copy is kept)
        assert_eq!(merge_alias_progress(&home), 1);
        let own = Progress::open(&dir.join(".courseshelf")).unwrap();
        assert_eq!(own.all("Provider/Course").unwrap().get("001 - Lecture 1").map(|p| p.position), Some(886.0));
        assert_eq!(merge_alias_progress(&home), 0, "a second run changes nothing");
        let _ = std::fs::remove_dir_all(&base);
    }
}
