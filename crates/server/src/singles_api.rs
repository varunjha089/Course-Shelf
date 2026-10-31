//! Single YouTube videos: paste a link, save it, download it where you choose.


use crate::jobs::JobInfo;
use crate::{err500, App};
use anyhow::{anyhow, bail, Result};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use courseshelf_core::fetcher::{self, Event};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};

type Shared = Arc<App>;
use crate::manage::*;

// ---------- single videos: paste a YouTube link, save it, download it where you choose ----------

pub async fn api_singles(State(app): State<Shared>) -> Json<serde_json::Value> {
    let s = courseshelf_core::singles::load(&app.home);
    Json(serde_json::json!({ "items": s.items, "last_dest": s.last_dest }))
}

#[derive(Deserialize)]
pub struct SingleAdd {
    url: String,
}

pub async fn api_single_add(State(app): State<Shared>, Json(r): Json<SingleAdd>) -> Response {
    use courseshelf_core::singles;
    let home = app.home.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<singles::Single> {
        let (kind, id) = singles::parse_url(&r.url)?;
        let mut st = singles::load(&home);
        let key = format!("{}-{}", if kind == "video" { "v" } else { "p" }, id);
        if let Some(x) = st.items.iter().find(|x| x.id == key) {
            bail!("already saved: {}", x.title);
        }
        let item = singles::lookup(&r.url)?;
        st.items.push(item.clone());
        singles::save(&home, &st)?;
        Ok(item)
    })
    .await;
    match res {
        Ok(Ok(i)) => Json(i).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
pub struct SingleId {
    id: String,
}

/// Forget a saved link. Never deletes a downloaded file.
pub async fn api_single_remove(State(app): State<Shared>, Json(r): Json<SingleId>) -> Response {
    use courseshelf_core::singles;
    let mut st = singles::load(&app.home);
    let n = st.items.len();
    st.items.retain(|x| x.id != r.id);
    if st.items.len() == n {
        return bad("no such saved link");
    }
    match singles::save(&app.home, &st) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
pub struct SingleDl {
    id: String,
    dest: String,
    #[serde(default)]
    create: bool,
}

pub async fn api_single_download(State(app): State<Shared>, Json(r): Json<SingleDl>) -> Response {
    use courseshelf_core::singles;
    let st = singles::load(&app.home);
    let Some(item) = st.items.iter().find(|x| x.id == r.id).cloned() else { return bad("no such saved link") };
    let dest = PathBuf::from(r.dest.trim());
    if !dest.is_absolute() {
        return bad("choose a folder (an absolute path)");
    }
    if dest.exists() && !dest.is_dir() {
        return bad(format!("{} is not a folder", dest.display()));
    }
    if !dest.exists() {
        // a missing folder is created only when asked, and only if its parent exists (so an unplugged disk is not faked)
        let parent_ok = dest.parent().map(|p| p.is_dir()).unwrap_or(false);
        if !r.create || !parent_ok {
            return bad(format!("{} does not exist{}", dest.display(), if r.create { " and its parent folder does not exist either (is the disk connected?)" } else { "; tick “create the folder” to make it" }));
        }
    }
    let cfg = match load_all(&app) {
        Ok((c, _)) => c,
        Err(e) => return bad(format!("{e:#}")),
    };
    let home = app.home.clone();
    let id = item.id.clone();
    let info = JobInfo {
        kind: "single".into(),
        course: format!("single:{}", item.id),
        title: item.title.clone(),
        total: item.count.max(1),
        ..Default::default()
    };
    let work: crate::jobs::Work = Box::new(move |ctx| {
        fetcher::require_tools()?;
        std::fs::create_dir_all(&dest)?;
        let probe = dest.join(".courseshelf_write_test");
        std::fs::write(&probe, b"x").map_err(|e| anyhow!("cannot write to {}: {e}", dest.display()))?;
        let _ = std::fs::remove_file(&probe);
        let dest = dest.canonicalize()?;
        // remember the folder for the next download, even if this one fails
        let mut s0 = singles::load(&home);
        s0.last_dest = Some(dest.to_string_lossy().to_string());
        singles::save(&home, &s0)?;
        let already = singles::existing(&dest, &item);
        let files = if !already.is_empty() {
            ctx.log(format!("already in this folder: {}", already[0].display()));
            ctx.update(|i| i.done = 1);
            already
        } else {
            let mut last = String::new();
            let mut sink = |ev: Event| match ev {
                Event::Line(l) => {
                    if l != last {
                        ctx.log(l.clone());
                        last = l;
                    }
                }
                Event::Item { index, title } => ctx.update(|i| {
                    i.current_index = index;
                    i.current_title = title;
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
                        i.failed.push(format!("{index}: {message}"));
                    }
                    i.percent = 0.0;
                }),
            };
            singles::download(&cfg, &item, &dest, &mut sink, &ctx.cancel)?
        };
        // save: the record keeps where the file went
        let mut s = singles::load(&home);
        if let Some(x) = s.items.iter_mut().find(|x| x.id == id) {
            x.status = "downloaded".into();
            x.dest = Some(dest.to_string_lossy().to_string());
            x.files = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
            x.downloaded_at = singles::now();
        }
        singles::save(&home, &s)?;
        Ok(())
    });
    match app.jobs.submit(info, work) {
        Ok(id) => Json(serde_json::json!({ "job": id })).into_response(),
        Err(e) => bad(e),
    }
}
