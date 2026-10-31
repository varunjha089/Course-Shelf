//! Moving a course to another drive or a chosen folder, and keeping a copy on this Mac (copy, later remove).


use crate::jobs::JobInfo;
use crate::{err500, App};
use anyhow::{anyhow, bail, Result};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use courseshelf_core::{config::Config, library, locations, mover, progress::Progress, spec};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

type Shared = Arc<App>;
use crate::manage::*;

// ---------- move to another drive ----------

#[derive(Deserialize)]
pub struct MoveReq {
    course: String,
    to_drive: String,
    /// Which copy to move when the course exists on more than one drive (courses without a row in courses.csv).
    #[serde(default)]
    from_drive: Option<String>,
    #[serde(default)]
    keep_original: bool,
    /// Move into exactly this folder instead of `<to_drive>/<course>` (it may be new; its parent must exist).
    #[serde(default)]
    to_folder: Option<String>,
}

fn move_plan(app: &App, course: &str, to_drive: &str, from_drive: Option<&str>, to_folder: Option<&str>) -> Result<(mover::MovePlan, Config, CourseRef, courseshelf_core::config::Drive)> {
    let (cfg, rows) = load_all(app)?;
    let cref = resolve_course(app, &cfg, &rows, course, from_drive)?;
    let (to, dst) = match to_folder.filter(|f| !f.is_empty()) {
        Some(f) => {
            let row = rows.iter().find(|r| r.course == cref.name).ok_or_else(|| anyhow!("a course without a row in courses.csv can only be moved to a drive"))?;
            let (loc, host) = exact_located(&cfg, row, f)?;
            if loc.dir.canonicalize().ok() == cref.dir.canonicalize().ok() {
                bail!("the course is already in {}", loc.dir.display());
            }
            if !library::scan_folder(&loc.dir).is_empty() {
                bail!("{} already holds videos; pick an empty or new folder", loc.dir.display());
            }
            (host, loc.dir)
        }
        None => {
            let to = cfg.drive(to_drive)?.clone();
            if !to.connected() {
                bail!("drive '{to_drive}' is not connected");
            }
            let dst = to.root.join(&cref.name);
            (to, dst)
        }
    };
    if dst == cref.dir {
        bail!("the course already lives on '{to_drive}'");
    }
    let plan = mover::plan(&cref.dir, &dst)?;
    Ok((plan, cfg, cref, to))
}

pub async fn api_move_plan(State(app): State<Shared>, Json(r): Json<MoveReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (plan, _cfg, _cref, to) = move_plan(&app, &r.course, &r.to_drive, r.from_drive.as_deref(), r.to_folder.as_deref())?;
        let free = to.space().map(|s| s.0);
        let need = if plan.same_fs && !r.keep_original { 0 } else { plan.total_bytes };
        let ok = free.map(|f| need as f64 * 1.02 <= f as f64).unwrap_or(true);
        Ok(serde_json::json!({
            "from": plan.src.to_string_lossy(), "to": plan.dst.to_string_lossy(),
            "files": plan.files.len(), "total_bytes": plan.total_bytes, "same_fs": plan.same_fs,
            "already_there": plan.already_there, "free_bytes": free, "ok": ok,
            "reason": if ok { "" } else { "not enough free space on the destination" },
        }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

pub async fn api_move_start(State(app): State<Shared>, Json(r): Json<MoveReq>) -> Response {
    let app2 = app.clone();
    let (course, to_drive, keep, from, to_folder) = (r.course.clone(), r.to_drive.clone(), r.keep_original, r.from_drive.clone(), r.to_folder.clone());
    let planned = tokio::task::spawn_blocking(move || move_plan(&app2, &course, &to_drive, from.as_deref(), to_folder.as_deref())).await;
    let (plan, cfg, cref, to) = match planned {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return bad(format!("{e:#}")),
        Err(e) => return err500(anyhow!("{e}")),
    };
    let need = if plan.same_fs && !keep { 0 } else { plan.total_bytes };
    if let Some((free, _)) = to.space() {
        if need as f64 * 1.02 > free as f64 {
            return bad("not enough free space on the destination");
        }
    }
    let info = JobInfo {
        kind: "move".into(),
        course: cref.name.clone(),
        drive: to.name.clone(),
        title: format!("{} → {}", nice(&cref.name), to.name),
        total: plan.files.len() as u32,
        bytes_total: plan.total_bytes,
        ..Default::default()
    };
    let home = app.home.clone();
    // the course's usual folder (courses.csv drive) and whether the move lands exactly there
    let usual = cref.csv_drive.as_deref().and_then(|d| cfg.drive(d).ok()).map(|d| (d.state_dir(), d.root.join(&cref.name)));
    let at_usual = usual.as_ref().is_none_or(|(_, dir)| *dir == plan.dst);
    let work: crate::jobs::Work = Box::new(move |ctx| {
        if cfg.drives.iter().any(|d| d.name == to.name && d.root == to.root) {
            to.init()?;
        }
        let mut files_done = 0u32;
        let mut cur = String::new();
        ctx.log(format!("{} {} file(s), {} MB: {} → {}", if plan.same_fs && !keep { "moving" } else { "copying" }, plan.files.len(), plan.total_bytes / 1_000_000, plan.src.display(), plan.dst.display()));
        mover::run(&plan, keep, &ctx.cancel, &mut |done, total, file| {
            if file != cur {
                if !cur.is_empty() {
                    files_done += 1;
                }
                cur = file.to_string();
            }
            ctx.update(|i| {
                i.bytes_done = done;
                i.bytes_total = total;
                i.done = files_done;
                i.current_title = file.to_string();
                i.percent = if total > 0 { done as f32 * 100.0 / total as f32 } else { 100.0 };
            });
        })?;
        ctx.update(|i| {
            i.done = i.total;
            i.bytes_done = i.bytes_total;
            i.percent = 100.0;
        });
        // progress follows the files
        let src_state = cref.state_dir.clone();
        // progress lives with the course: in the drive's state folder when the course sits at its usual place on the drive
        // courses.csv names (or is not in the CSV at all and lands on a drive), otherwise beside the course folder, as `locate` finds it
        let at_home_drive = at_usual;
        if src_state.join("progress.sqlite").exists() {
            let from = Progress::open(&src_state)?;
            let dest_state = if at_home_drive { usual.as_ref().map_or_else(|| to.state_dir(), |(s, _)| s.clone()) } else { plan.dst.join(".courseshelf") };
            let into = Progress::open(&dest_state)?;
            let rows = from.export(&cref.name)?;
            let n = into.import(&cref.name, &rows, Some(from.speed(&cref.name)))?;
            ctx.log(format!("carried over progress for {n} lecture(s)"));
        }
        // a course with a CSV row needs an explicit location only when it now sits on a drive the CSV does not name
        if cref.csv_drive.is_some() {
            if at_home_drive {
                locations::clear(&home, &cref.name)?;
            } else {
                locations::set(&home, &cref.name, &plan.dst)?;
            }
        }
        ctx.log("done: the app now reads this course from the new place");
        Ok(())
    });
    match app.jobs.submit(info, work) {
        Ok(id) => Json(serde_json::json!({ "job": id })).into_response(),
        Err(e) => bad(e),
    }
}

// ---------- keep a course on this Mac (copy from the HDD, free it later) ----------

#[derive(Deserialize)]
pub struct MacReq {
    course: String,
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1e9)
}

/// Where a course can be read from other than this Mac: an explicit location, else any other connected drive's folder.
fn other_copy(cfg: &Config, home: &Path, mac_root: &Path, course: &str) -> Option<(PathBuf, PathBuf, String)> {
    if let Some(p) = locations::load(home).get(course) {
        if p.is_dir() && !p.starts_with(mac_root) && !library::scan_folder(p).is_empty() {
            return Some((p.clone(), p.join(".courseshelf"), "custom folder".into()));
        }
    }
    for d in cfg.drives.iter().filter(|d| d.connected() && d.root != mac_root) {
        let dir = d.root.join(course);
        if dir.is_dir() && !library::scan_folder(&dir).is_empty() {
            return Some((dir, d.state_dir(), d.name.clone()));
        }
    }
    None
}

pub async fn api_mac_copy(State(app): State<Shared>, Json(r): Json<MacReq>) -> Response {
    let app2 = app.clone();
    let course = r.course.clone();
    let planned = tokio::task::spawn_blocking(move || -> Result<(mover::MovePlan, PathBuf, courseshelf_core::config::Drive, String)> {
        spec::check_course_name(&course)?;
        let cfg = Config::load(&app2.home)?;
        let mac = cfg.mac_drive().ok_or_else(|| anyhow!("no drive is marked as this Mac in config.toml ([mac] drive = \"local\")"))?.clone();
        if !mac.connected() {
            bail!("the Mac's course folder {} is not available", mac.root.display());
        }
        let (src, src_state, from) = other_copy(&cfg, &app2.home, &mac.root, &course).ok_or_else(|| anyhow!("no connected copy to copy from: plug in the drive that holds this course"))?;
        let dst = mac.root.join(&course);
        let plan = mover::plan(&src, &dst)?;
        let to_copy = plan.bytes_to_copy();
        let used: u64 = library::scan_root(&mac.root).iter().flat_map(|c| c.lectures.iter()).map(|l| l.size_bytes).sum();
        if let Some(max) = cfg.mac_max_bytes() {
            if used + to_copy > max {
                bail!("not enough room in your Mac limit of {}: courses on the Mac use {}, and copying this one needs {} more. Free some space first (Remove from Mac on a course you've finished) or raise max_gb under [mac] in config.toml.", gb(max), gb(used), gb(to_copy));
            }
        }
        if let Some((free, _)) = mac.space() {
            if to_copy as f64 * 1.05 > free as f64 {
                bail!("the Mac disk has only {} free, and this copy needs {}", gb(free), gb(to_copy));
            }
        }
        Ok((plan, src_state, mac, from))
    })
    .await;
    let (plan, src_state, mac, from) = match planned {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return bad(format!("{e:#}")),
        Err(e) => return err500(anyhow!("{e}")),
    };
    let info = JobInfo {
        kind: "move".into(),
        course: r.course.clone(),
        drive: mac.name.clone(),
        title: format!("Copy to Mac: {}", nice(&r.course)),
        total: plan.files.len() as u32,
        bytes_total: plan.total_bytes,
        ..Default::default()
    };
    let course = r.course.clone();
    let work: crate::jobs::Work = Box::new(move |ctx| {
        mac.init()?;
        let mut files_done = 0u32;
        let mut cur = String::new();
        ctx.log(format!("copying {} file(s), {} from {} to this Mac ({})", plan.files.len(), gb(plan.total_bytes), from, plan.dst.display()));
        mover::run(&plan, true, &ctx.cancel, &mut |done, total, file| {
            if file != cur {
                if !cur.is_empty() {
                    files_done += 1;
                }
                cur = file.to_string();
            }
            ctx.update(|i| {
                i.bytes_done = done;
                i.bytes_total = total;
                i.done = files_done;
                i.current_title = file.to_string();
                i.percent = if total > 0 { done as f32 * 100.0 / total as f32 } else { 100.0 };
            });
        })?;
        ctx.update(|i| {
            i.done = i.total;
            i.bytes_done = i.bytes_total;
            i.percent = 100.0;
        });
        // carry what you have watched to the Mac copy (the newest record wins in both places)
        if src_state.join("progress.sqlite").exists() {
            let from_db = Progress::open(&src_state)?;
            let into = Progress::open(&mac.state_dir())?;
            let n = into.import(&course, &from_db.export(&course)?, Some(from_db.speed(&course)))?;
            ctx.log(format!("carried over progress for {n} lecture(s)"));
        }
        ctx.log("done: the app now plays this course from the Mac; the original copy was not touched");
        Ok(())
    });
    match app.jobs.submit(info, work) {
        Ok(id) => Json(serde_json::json!({ "job": id })).into_response(),
        Err(e) => bad(e),
    }
}

/// Delete the Mac copy of a course, but only after every file has been checked against the other copy.
pub async fn api_mac_remove(State(app): State<Shared>, Json(r): Json<MacReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        spec::check_course_name(&r.course)?;
        let cfg = Config::load(&app.home)?;
        let mac = cfg.mac_drive().ok_or_else(|| anyhow!("no drive is marked as this Mac in config.toml"))?.clone();
        let mac_dir = mac.root.join(&r.course);
        if !mac_dir.is_dir() {
            bail!("there is no Mac copy of this course");
        }
        let (other, other_state, from) = other_copy(&cfg, &app.home, &mac.root, &r.course)
            .ok_or_else(|| anyhow!("the other copy is not connected. Plug in the drive that holds this course: the Mac copy is only removed after it has been checked against that copy"))?;
        if mac_dir.canonicalize().ok() == other.canonicalize().ok() {
            bail!("both copies are the same folder");
        }
        // every real file on the Mac must exist, with the same size and content sample, in the other copy
        let mut files: Vec<(PathBuf, u64)> = Vec::new();
        let mut problems: Vec<String> = Vec::new();
        for e in std::fs::read_dir(&mac_dir)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let md = e.metadata()?;
            if name.starts_with('.') || !md.is_file() {
                continue;
            }
            let o = other.join(&name);
            match std::fs::metadata(&o) {
                Ok(om) if om.len() == md.len() => {
                    if mover::sample_hash(&e.path())? != mover::sample_hash(&o)? {
                        problems.push(format!("{name}: content differs from the copy on {from}"));
                    }
                }
                Ok(_) => problems.push(format!("{name}: size differs from the copy on {from}")),
                Err(_) => problems.push(format!("{name}: missing from the copy on {from}")),
            }
            files.push((e.path(), md.len()));
        }
        if !problems.is_empty() {
            bail!("the Mac copy was NOT removed because the copy on {from} does not match it ({} problem(s)), e.g. {}", problems.len(), problems.iter().take(3).cloned().collect::<Vec<_>>().join("; "));
        }
        // keep your watch positions: merge the Mac database into the other copy's before anything is deleted
        let mac_state = mac.state_dir();
        if mac_state.join("progress.sqlite").exists() {
            let from_db = Progress::open(&mac_state)?;
            let into = Progress::open(&other_state)?;
            into.import(&r.course, &from_db.export(&r.course)?, Some(from_db.speed(&r.course)))?;
        }
        let freed: u64 = files.iter().map(|f| f.1).sum();
        for (p, _) in &files {
            std::fs::remove_file(p)?;
        }
        let _ = std::fs::remove_file(mac_dir.join(".DS_Store"));
        let _ = std::fs::remove_dir(&mac_dir); // only succeeds when nothing else is in it
        Ok(serde_json::json!({ "removed": files.len(), "freed_bytes": freed, "kept_copy_on": from }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}
