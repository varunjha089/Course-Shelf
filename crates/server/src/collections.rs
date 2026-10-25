//! Book collections: several papers joined into one PDF you read and annotate as a single book.
//!
//! A collection is `collections/<id>.pdf` (made with `pdfunite`) plus an entry in `collections.json` that records
//! the title the user chose and where each paper starts. Marks are stored under the collection's own id (not a hash
//! of the file), so adding papers later (appended at the end) never moves or loses a mark. Papers that are not on
//! disk are downloaded into the inbox when they are added; that is the user's explicit request for this feature.

use crate::{download_blocking, err500, paper_file, papers_paths, pdf_pages, safe_join, serve_pdf, App};
use anyhow::{anyhow, Result};
use axum::{
    body::Body,
    extract::{Path as AxPath, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use courseshelf_core::papers;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Shared = Arc<App>;

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CPaper {
    pub paper_id: String,
    pub title: String,
    pub pages: u32,
    /// first page of this paper inside the book (1-based)
    pub start: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Coll {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub papers: Vec<CPaper>,
}

fn manifest(home: &Path) -> PathBuf {
    home.join("collections.json")
}

fn pdf_path(home: &Path, id: &str) -> PathBuf {
    home.join("collections").join(format!("{id}.pdf"))
}

fn lock() -> &'static Mutex<()> {
    static L: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn load(home: &Path) -> Vec<Coll> {
    std::fs::read_to_string(manifest(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(home: &Path, v: &[Coll]) -> Result<()> {
    let tmp = home.join(".collections.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, manifest(home))?;
    Ok(())
}

/// The merged PDF of a collection, if it exists.
pub(crate) fn collection_file(app: &App, id: &str) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    load(&app.home).iter().any(|c| c.id == id).then(|| pdf_path(&app.home, id)).filter(|p| p.is_file())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn bad(msg: impl Into<String>) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, msg.into()).into_response()
}

/// A paper's PDF on disk: the library copy, else the inbox copy.
fn local_pdf(app: &App, id: &str) -> Option<PathBuf> {
    // a 16-hex id is a PDF from a [pdf_sources] folder (a "book" in the Books tab), not a database paper
    if id.len() == 16 {
        return crate::annot::book_file(app, id);
    }
    if let Some(p) = paper_file(app, id) {
        return Some(p);
    }
    let pp = papers_paths(app).ok()?;
    let rec = papers::load_inbox(&pp.inbox).into_iter().find(|r| r.id == id)?;
    safe_join(&pp.inbox, &rec.file).filter(|p| p.is_file())
}

fn paper_title(app: &App, id: &str) -> String {
    if id.len() == 16 {
        if let Some(p) = crate::annot::book_file(app, id) {
            let stem = p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            let t = stem.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches('_');
            return t.replace('_', " ").trim().to_string();
        }
    }
    papers_paths(app)
        .ok()
        .and_then(|pp| crate::load_list(app, &pp).ok())
        .and_then(|l| l.iter().find(|p| p.id == id).map(|p| p.title.clone()))
        .unwrap_or_else(|| id.to_string())
}

/// Make or extend a collection. `existing` is Some when adding to one (new papers go after the old pages).
fn build(app: &App, ctx: &crate::jobs::JobCtx, id: &str, title: &str, ids: &[String], existing: Option<Coll>) -> Result<()> {
    let have: Vec<String> = existing.as_ref().map(|c| c.papers.iter().map(|p| p.paper_id.clone()).collect()).unwrap_or_default();
    let todo: Vec<&String> = ids.iter().filter(|i| !have.contains(i)).collect();
    if todo.is_empty() {
        return Err(anyhow!("those papers are already in this book"));
    }
    ctx.update(|i| i.total = todo.len() as u32);
    let mut got: Vec<(String, String, PathBuf, u32)> = vec![];
    for (n, pid) in todo.iter().enumerate() {
        if ctx.cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(anyhow!("cancelled"));
        }
        let t = paper_title(app, pid);
        ctx.update(|i| {
            i.current_index = n as u32 + 1;
            i.current_title = t.clone();
        });
        let path = match local_pdf(app, pid) {
            Some(p) => Some(p),
            None => {
                ctx.log(format!("downloading: {t}"));
                let r = download_blocking(app, pid);
                std::thread::sleep(std::time::Duration::from_secs(3)); // arXiv asks for polite use
                match r {
                    Ok(rec) => papers_paths(app).ok().and_then(|pp| safe_join(&pp.inbox, &rec.file)),
                    Err(e) => {
                        ctx.update(|i| i.warnings.push(format!("{t}: not added ({e:#})")));
                        None
                    }
                }
            }
        };
        if let Some(p) = path {
            let pages = pdf_pages(&p);
            if pages == 0 {
                ctx.update(|i| i.warnings.push(format!("{t}: not added (could not read its pages)")));
            } else {
                got.push(((*pid).clone(), t, p, pages));
            }
        }
        ctx.update(|i| i.done = n as u32 + 1);
    }
    if got.is_empty() {
        return Err(anyhow!("none of the selected papers could be added"));
    }
    let dir = app.home.join("collections");
    std::fs::create_dir_all(&dir)?;
    let out = pdf_path(&app.home, id);
    let tmp = dir.join(format!(".{id}.part.pdf"));
    let mut inputs: Vec<PathBuf> = vec![];
    if existing.is_some() {
        inputs.push(out.clone());
    }
    inputs.extend(got.iter().map(|g| g.2.clone()));
    if inputs.len() == 1 {
        std::fs::copy(&inputs[0], &tmp)?;
    } else {
        if courseshelf_core::tools::find("pdfunite").is_none() {
            return Err(anyhow!("pdfunite is not installed (it comes with poppler: brew install poppler)"));
        }
        ctx.log("joining the PDFs");
        let o = courseshelf_core::tools::command("pdfunite").args(&inputs).arg(&tmp).output()?;
        if !o.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("could not join the PDFs: {}", String::from_utf8_lossy(&o.stderr).trim()));
        }
    }
    std::fs::rename(&tmp, &out)?;
    let _g = lock().lock().map_err(|_| anyhow!("collections lock poisoned"))?;
    let mut all = load(&app.home);
    let mut c = existing.unwrap_or(Coll { id: id.to_string(), title: title.to_string(), created: now(), papers: vec![] });
    let mut next = c.papers.last().map(|p| p.start + p.pages).unwrap_or(1);
    for (pid, t, _, pages) in got {
        c.papers.push(CPaper { paper_id: pid, title: t, pages, start: next });
        next += pages;
    }
    all.retain(|x| x.id != c.id);
    all.push(c);
    save(&app.home, &all)?;
    ctx.log("done");
    Ok(())
}

#[derive(Deserialize)]
struct CreateReq {
    title: String,
    paper_ids: Vec<String>,
}

fn submit(app: &Shared, id: String, title: String, ids: Vec<String>, existing: Option<Coll>) -> Response {
    let info = crate::jobs::JobInfo { kind: "collection".into(), course: format!("collection:{id}"), title: title.clone(), ..Default::default() };
    let a = app.clone();
    let work: crate::jobs::Work = Box::new(move |ctx| build(&a, ctx, &id, &title, &ids, existing));
    match app.jobs.submit(info, work) {
        Ok(j) => Json(serde_json::json!({ "job": j })).into_response(),
        Err(e) => bad(e),
    }
}

async fn api_create(State(app): State<Shared>, Json(r): Json<CreateReq>) -> Response {
    let title = r.title.trim().to_string();
    if title.is_empty() {
        return bad("give the book a title");
    }
    if r.paper_ids.is_empty() {
        return bad("select at least one paper");
    }
    if load(&app.home).iter().any(|c| c.title.eq_ignore_ascii_case(&title)) {
        return bad("a book with that title already exists");
    }
    match courseshelf_core::random_hex(8) {
        Ok(id) => submit(&app, id, title, r.paper_ids, None),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
struct AddReq {
    id: String,
    paper_ids: Vec<String>,
}

async fn api_add(State(app): State<Shared>, Json(r): Json<AddReq>) -> Response {
    let Some(c) = load(&app.home).into_iter().find(|c| c.id == r.id) else { return bad("unknown book") };
    submit(&app, c.id.clone(), c.title.clone(), r.paper_ids, Some(c))
}

#[derive(Deserialize)]
struct RenameReq {
    id: String,
    title: String,
}

async fn api_rename(State(app): State<Shared>, Json(r): Json<RenameReq>) -> Response {
    let title = r.title.trim().to_string();
    if title.is_empty() {
        return bad("give the book a title");
    }
    let _g = lock().lock().unwrap();
    let mut all = load(&app.home);
    if all.iter().any(|c| c.id != r.id && c.title.eq_ignore_ascii_case(&title)) {
        return bad("a book with that title already exists");
    }
    let Some(c) = all.iter_mut().find(|c| c.id == r.id) else { return bad("unknown book") };
    c.title = title;
    match save(&app.home, &all) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
struct ReorderReq {
    id: String,
    /// paper ids in the wanted reading order; any paper left out is removed from the book
    order: Vec<String>,
}

/// Re-join the book in a new order (or without some papers). Marks and bookmarks follow their paper to its new
/// pages; marks on a removed paper are dropped, after the old marks file is copied to `<id>.json.bak_<time>`.
fn reorder(app: &App, id: &str, order: &[String]) -> Result<()> {
    let _g = lock().lock().map_err(|_| anyhow!("collections lock poisoned"))?;
    let mut all = load(&app.home);
    let Some(pos) = all.iter().position(|c| c.id == id) else { return Err(anyhow!("unknown book")) };
    let old = all[pos].clone();
    if order.is_empty() {
        return Err(anyhow!("a book needs at least one paper (delete the book instead)"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut chosen: Vec<&CPaper> = vec![];
    for pid in order {
        let Some(p) = old.papers.iter().find(|p| &p.paper_id == pid) else { return Err(anyhow!("{pid} is not in this book")) };
        if !seen.insert(pid.clone()) {
            return Err(anyhow!("{pid} is listed twice"));
        }
        chosen.push(p);
    }
    let mut files = vec![];
    for p in &chosen {
        let f = local_pdf(app, &p.paper_id).ok_or_else(|| anyhow!("{}: its PDF is not on disk any more", p.title))?;
        if pdf_pages(&f) != p.pages {
            return Err(anyhow!("{}: the PDF on disk has a different page count than when the book was made", p.title));
        }
        files.push(f);
    }
    let out = pdf_path(&app.home, id);
    let tmp = app.home.join("collections").join(format!(".{id}.part.pdf"));
    if files.len() == 1 {
        std::fs::copy(&files[0], &tmp)?;
    } else {
        if courseshelf_core::tools::find("pdfunite").is_none() {
            return Err(anyhow!("pdfunite is not installed (it comes with poppler: brew install poppler)"));
        }
        let o = courseshelf_core::tools::command("pdfunite").args(&files).arg(&tmp).output()?;
        if !o.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("could not join the PDFs: {}", String::from_utf8_lossy(&o.stderr).trim()));
        }
    }
    // new start page of each kept paper
    let mut next = 1u32;
    let mut newp: Vec<CPaper> = vec![];
    for p in &chosen {
        newp.push(CPaper { paper_id: p.paper_id.clone(), title: p.title.clone(), pages: p.pages, start: next });
        next += p.pages;
    }
    let map = |page: u64| -> Option<u64> {
        let o = old.papers.iter().find(|p| page >= p.start as u64 && page < (p.start + p.pages) as u64)?;
        let n = newp.iter().find(|p| p.paper_id == o.paper_id)?;
        Some(n.start as u64 + (page - o.start as u64))
    };
    // marks first, then the PDF: if the marks step fails, the old book is untouched
    let af = app.home.join("annotations").join(format!("{id}.json"));
    if let Some(mut doc) = std::fs::read_to_string(&af).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        std::fs::copy(&af, af.with_extension(format!("json.bak_{}", now())))?;
        if let Some(els) = doc.get_mut("els").and_then(|v| v.as_array_mut()) {
            els.retain_mut(|e| match e.get("page").and_then(|v| v.as_u64()).and_then(&map) {
                Some(n) => {
                    e["page"] = n.into();
                    true
                }
                None => false,
            });
        }
        if let Some(b) = doc.get_mut("bookmarks").and_then(|v| v.as_array_mut()) {
            let kept: Vec<serde_json::Value> = b.iter().filter_map(|v| v.as_u64().and_then(&map)).map(|n| n.into()).collect();
            *b = kept;
        }
        let vp = doc.pointer("/view/page").and_then(|v| v.as_u64()).and_then(&map).unwrap_or(1);
        if let Some(v) = doc.get_mut("view") {
            v["page"] = vp.into();
        }
        let t = af.with_extension("json.tmp");
        std::fs::write(&t, serde_json::to_vec(&doc)?)?;
        std::fs::rename(&t, &af)?;
    }
    std::fs::rename(&tmp, &out)?;
    all[pos].papers = newp;
    save(&app.home, &all)
}

async fn api_reorder(State(app): State<Shared>, Json(r): Json<ReorderReq>) -> Response {
    if !valid_id(&r.id) {
        return bad("bad id");
    }
    match tokio::task::spawn_blocking(move || reorder(&app, &r.id, &r.order)).await {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
struct IdReq {
    id: String,
}

/// Forget the book and its joined PDF. The papers themselves and the marks (annotations/<id>.json) are kept.
async fn api_delete(State(app): State<Shared>, Json(r): Json<IdReq>) -> Response {
    if !valid_id(&r.id) {
        return bad("bad id");
    }
    let _g = lock().lock().unwrap();
    let mut all = load(&app.home);
    if !all.iter().any(|c| c.id == r.id) {
        return bad("unknown book");
    }
    all.retain(|c| c.id != r.id);
    if let Err(e) = save(&app.home, &all) {
        return err500(e);
    }
    let _ = std::fs::remove_file(pdf_path(&app.home, &r.id));
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn api_list(State(app): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "collections": load(&app.home) }))
}

async fn pdf_collection(State(app): State<Shared>, AxPath(id): AxPath<String>, req: Request<Body>) -> Response {
    match collection_file(&app, &id) {
        Some(p) => serve_pdf(p, req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/collections", get(api_list))
        .route("/api/collections/create", post(api_create))
        .route("/api/collections/add", post(api_add))
        .route("/api/collections/rename", post(api_rename))
        .route("/api/collections/delete", post(api_delete))
        .route("/api/collections/reorder", post(api_reorder))
        .route("/pdf/collection/{id}", get(pdf_collection))
}
