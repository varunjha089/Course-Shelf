//! Books and PDF annotations. The Books tab lists every PDF under the folders named in `[pdf_sources]`;
//! annotations are stored apart from the PDF (never inside it), keyed by a hash of the file's bytes, so they
//! survive a rename, a move to another drive or a symlink to the same file.

use crate::{err500, paper_file, papers_paths, safe_join, serve_pdf, App};
use anyhow::anyhow;
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path as AxPath, Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use courseshelf_core::{config::Config, mover, papers};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

type Shared = Arc<App>;

fn bad(msg: impl Into<String>) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, msg.into()).into_response()
}

fn scan(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if out.len() >= 20000 {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
            if depth < 5 {
                scan(&p, depth + 1, out);
            }
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("pdf")).unwrap_or(false) {
            out.push(p);
        }
    }
}

fn book_id(p: &Path) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in p.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn cache() -> &'static Mutex<HashMap<String, PathBuf>> {
    static C: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Serialize)]
struct FolderOut {
    path: String,
    connected: bool,
    count: usize,
}

#[derive(Serialize)]
struct BookOut {
    id: String,
    name: String,
    folder: String,
    rel: String,
    size: u64,
    bookmarked: bool,
}

/// Bookmarked books: full paths in bookmarks.json (next to config.toml). Paths, not ids, so the file stays readable.
fn bookmarks_path(home: &Path) -> PathBuf {
    home.join("bookmarks.json")
}

fn load_bookmarks(home: &Path) -> Vec<String> {
    std::fs::read_to_string(bookmarks_path(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_bookmarks(home: &Path, v: &[String]) -> anyhow::Result<()> {
    let f = bookmarks_path(home);
    let tmp = home.join(".bookmarks.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, &f)?;
    Ok(())
}

fn list(cfg: &Config, marks: &[String]) -> (Vec<FolderOut>, Vec<BookOut>) {
    let mut folders = vec![];
    let mut books = vec![];
    let mut map = HashMap::new();
    for f in &cfg.pdf_folders {
        let mut files = vec![];
        let connected = f.is_dir();
        if connected {
            scan(f, 0, &mut files);
        }
        folders.push(FolderOut { path: f.to_string_lossy().to_string(), connected, count: files.len() });
        for p in files {
            let id = book_id(&p);
            let rel = p.strip_prefix(f).unwrap_or(&p).to_string_lossy().to_string();
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            map.insert(id.clone(), p.clone());
            let bookmarked = marks.iter().any(|m| m == &p.to_string_lossy());
            books.push(BookOut { id, name, folder: f.to_string_lossy().to_string(), rel, size, bookmarked });
        }
    }
    books.sort_by(|a, b| a.rel.to_lowercase().cmp(&b.rel.to_lowercase()));
    *cache().lock().unwrap() = map;
    (folders, books)
}

pub(crate) fn book_file(app: &App, id: &str) -> Option<PathBuf> {
    if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let hit = cache().lock().unwrap().get(id).cloned();
    if let Some(p) = hit.filter(|p| p.is_file()) {
        return Some(p);
    }
    let cfg = Config::load(&app.home).ok()?;
    list(&cfg, &[]);
    cache().lock().unwrap().get(id).cloned().filter(|p| p.is_file())
}

async fn api_books(State(app): State<Shared>) -> Response {
    let res = tokio::task::spawn_blocking(move || {
        let cfg = Config::load(&app.home)?;
        let (folders, books) = list(&cfg, &load_bookmarks(&app.home));
        let marks = load_bookmarks(&app.home);
        let colls: Vec<serde_json::Value> = crate::collections::load(&app.home)
            .into_iter()
            .map(|c| {
                let bookmarked = marks.contains(&format!("collection:{}", c.id));
                serde_json::json!({ "id": c.id, "title": c.title, "papers": c.papers, "bookmarked": bookmarked })
            })
            .collect();
        Ok::<_, anyhow::Error>(serde_json::json!({ "folders": folders, "books": books, "collections": colls }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => err500(e),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
struct MarkReq {
    id: String,
    on: bool,
}

async fn api_bookmark(State(app): State<Shared>, Json(r): Json<MarkReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let ps = if crate::collections::load(&app.home).iter().any(|c| c.id == r.id) {
            format!("collection:{}", r.id)
        } else {
            book_file(&app, &r.id).ok_or_else(|| anyhow!("that book is not available"))?.to_string_lossy().to_string()
        };
        let mut v = load_bookmarks(&app.home);
        v.retain(|m| m != &ps);
        if r.on {
            v.push(ps);
        }
        save_bookmarks(&app.home, &v)
    })
    .await;
    match res {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

async fn pdf_book(State(app): State<Shared>, AxPath(id): AxPath<String>, req: Request<Body>) -> Response {
    match book_file(&app, &id) {
        Some(p) => serve_pdf(p, req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The file behind a viewer link: a library paper, an inbox download, a question bank or a book.
pub(crate) fn resolve(app: &App, kind: &str, r: &str) -> Option<PathBuf> {
    match kind {
        "paper" => paper_file(app, r),
        "inbox" => {
            let pp = papers_paths(app).ok()?;
            let rec = papers::load_inbox(&pp.inbox).into_iter().find(|x| x.id == r)?;
            safe_join(&pp.inbox, &rec.file)
        }
        "bank" => {
            let pp = papers_paths(app).ok()?;
            safe_join(&pp.banks_dir, r)
        }
        "book" => book_file(app, r),
        "collection" => crate::collections::collection_file(app, r),
        _ => None,
    }
}

fn dir(app: &App) -> PathBuf {
    app.home.join("annotations")
}

fn valid_key(k: &str) -> bool {
    k.len() == 16 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Deserialize)]
struct LoadReq {
    kind: String,
    #[serde(rename = "ref")]
    r: String,
}

async fn api_load(State(app): State<Shared>, Json(r): Json<LoadReq>) -> Response {
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let p = resolve(&app, &r.kind, &r.r).ok_or_else(|| anyhow!("that PDF is not available (is its disk connected?)"))?;
        // a collection keeps its marks under its own id, so adding papers to it never loses them
        let key = if r.kind == "collection" { r.r.clone() } else { format!("{:016x}", mover::sample_hash(&p)?) };
        let doc = std::fs::read_to_string(dir(&app).join(format!("{key}.json"))).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        Ok(serde_json::json!({ "key": key, "doc": doc, "file": p.to_string_lossy() }))
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => bad(format!("{e:#}")),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
struct SaveReq {
    key: String,
    doc: serde_json::Value,
}

async fn api_save(State(app): State<Shared>, Json(r): Json<SaveReq>) -> Response {
    if !valid_key(&r.key) {
        return bad("bad key");
    }
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let d = dir(&app);
        std::fs::create_dir_all(&d)?;
        let f = d.join(format!("{}.json", r.key));
        let tmp = d.join(format!(".{}.json.tmp", r.key));
        std::fs::write(&tmp, serde_json::to_vec(&r.doc)?)?;
        std::fs::rename(&tmp, &f)?;
        Ok(())
    })
    .await;
    match res {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => err500(e),
        Err(e) => err500(anyhow!("{e}")),
    }
}

#[derive(Deserialize)]
struct ImgQ {
    ext: Option<String>,
}

async fn api_image_put(State(app): State<Shared>, AxPath(key): AxPath<String>, Query(q): Query<ImgQ>, body: Bytes) -> Response {
    let ext = q.ext.unwrap_or_else(|| "png".into()).to_ascii_lowercase();
    if !valid_key(&key) || !["png", "jpg", "jpeg", "webp", "gif"].contains(&ext.as_str()) || body.is_empty() {
        return bad("bad image request");
    }
    let res = (|| -> anyhow::Result<String> {
        let d = dir(&app).join(&key);
        std::fs::create_dir_all(&d)?;
        let id = courseshelf_core::random_hex(6)?;
        let file = format!("{id}.{ext}");
        std::fs::write(d.join(&file), &body)?;
        Ok(file)
    })();
    match res {
        Ok(file) => Json(serde_json::json!({ "file": file })).into_response(),
        Err(e) => err500(e),
    }
}

async fn api_image_get(State(app): State<Shared>, AxPath((key, file)): AxPath<(String, String)>, req: Request<Body>) -> Response {
    if !valid_key(&key) || file.contains('/') || file.contains("..") || file.starts_with('.') {
        return StatusCode::NOT_FOUND.into_response();
    }
    use tower::ServiceExt;
    match tower_http::services::ServeFile::new(dir(&app).join(&key).join(&file)).oneshot(req).await {
        Ok(r) => r.map(Body::new).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct ExportQ {
    name: String,
}

async fn api_export(State(app): State<Shared>, Query(q): Query<ExportQ>, body: Bytes) -> Response {
    if !body.starts_with(b"%PDF") {
        return bad("that is not a PDF");
    }
    let stem: String = q.name.trim_end_matches(".pdf").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(100).collect();
    let stem = if stem.is_empty() { "annotated".to_string() } else { stem };
    let res = (|| -> anyhow::Result<String> {
        let d = dir(&app).join("exports");
        std::fs::create_dir_all(&d)?;
        let file = format!("{stem}_annotated.pdf");
        std::fs::write(d.join(&file), &body)?;
        Ok(file)
    })();
    match res {
        Ok(file) => Json(serde_json::json!({ "file": file, "path": dir(&app).join("exports").join(&file) })).into_response(),
        Err(e) => err500(e),
    }
}

#[derive(Deserialize)]
struct RevealReq {
    file: String,
}

async fn api_reveal(State(app): State<Shared>, Json(r): Json<RevealReq>) -> Response {
    if r.file.contains('/') || r.file.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let p = dir(&app).join("exports").join(&r.file);
    if !p.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match courseshelf_core::tools::command("open").arg("-R").arg(&p).status() {
        Ok(s) if s.success() => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(s) => err500(anyhow!("open exited with {s}")),
        Err(e) => err500(e.into()),
    }
}

pub(crate) fn routes() -> Router<Shared> {
    let big = DefaultBodyLimit::max(256 << 20);
    Router::new()
        .route("/api/books", get(api_books))
        .route("/api/books/bookmark", post(api_bookmark))
        .route("/pdf/book/{id}", get(pdf_book))
        .route("/api/annot/load", post(api_load))
        .route("/api/annot/save", post(api_save).layer(big.clone()))
        .route("/api/annot/image/{key}", post(api_image_put).layer(big.clone()))
        .route("/api/annot/image/{key}/{file}", get(api_image_get))
        .route("/api/annot/export", post(api_export).layer(big))
        .route("/api/annot/reveal", post(api_reveal))
}
