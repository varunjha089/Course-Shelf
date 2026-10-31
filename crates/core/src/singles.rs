//! Single YouTube videos (or one playlist) saved from a pasted link: for videos that are not part of a course in courses.csv,
//! for example a freeCodeCamp lecture. The links are kept in `singles.json` next to config.toml. Nothing is downloaded until
//! the user presses Download and has chosen the folder.

use anyhow::{bail, Context, Result};
use crate::fetcher::{self, Event};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::{AtomicBool, Ordering}, mpsc},
    time::{Duration, SystemTime},
};

const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov"];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Single {
    pub id: String,
    /// "video" | "playlist"
    pub kind: String,
    pub yt_id: String,
    pub url: String,
    pub title: String,
    pub uploader: String,
    pub duration: Option<f64>,
    /// number of videos (1 for a single video)
    pub count: u32,
    pub added: u64,
    /// "saved" | "downloaded"
    pub status: String,
    pub dest: Option<String>,
    pub files: Vec<String>,
    pub downloaded_at: u64,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Store {
    pub items: Vec<Single>,
    pub last_dest: Option<String>,
}

fn file(home: &Path) -> PathBuf {
    home.join("singles.json")
}

pub fn load(home: &Path) -> Store {
    std::fs::read_to_string(file(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(home: &Path, s: &Store) -> Result<()> {
    let tmp = file(home).with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s)?)?;
    std::fs::rename(&tmp, file(home))?;
    Ok(())
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn query_param(q: &str, key: &str) -> Option<String> {
    q.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

/// ("video" | "playlist", id) from a YouTube address. Only YouTube hosts are accepted.
pub fn parse_url(url: &str) -> Result<(String, String)> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).context("the link must start with https://")?;
    let (hostpath, query) = rest.split_once('?').unwrap_or((rest, ""));
    let query = query.split('#').next().unwrap_or("");
    let hostpath = hostpath.split('#').next().unwrap_or("");
    let (host, path) = hostpath.split_once('/').unwrap_or((hostpath, ""));
    let host = host.to_ascii_lowercase();
    let path = path.trim_end_matches('/');
    if host == "youtu.be" {
        let id = path.split('/').next().unwrap_or("");
        if valid_id(id) {
            return Ok(("video".into(), id.into()));
        }
        bail!("could not find the video id in {u}");
    }
    if !(host == "youtube.com" || host.ends_with(".youtube.com")) {
        bail!("only YouTube links are supported (youtube.com or youtu.be)");
    }
    if let Some(v) = query_param(query, "v").filter(|v| valid_id(v)) {
        return Ok(("video".into(), v));
    }
    for pre in ["shorts/", "live/", "embed/", "v/"] {
        if let Some(id) = path.strip_prefix(pre).map(|x| x.split('/').next().unwrap_or("")).filter(|x| valid_id(x)) {
            return Ok(("video".into(), id.into()));
        }
    }
    if path == "playlist" {
        if let Some(l) = query_param(query, "list").filter(|l| valid_id(l)) {
            return Ok(("playlist".into(), l));
        }
    }
    bail!("this does not look like a YouTube video or playlist link")
}

fn canonical(kind: &str, id: &str) -> String {
    if kind == "video" { format!("https://www.youtube.com/watch?v={id}") } else { format!("https://www.youtube.com/playlist?list={id}") }
}

/// Look the link up (metadata only, nothing is downloaded) and build the record to save.
pub fn lookup(url: &str) -> Result<Single> {
    let (kind, yt_id) = parse_url(url)?;
    fetcher::require_tools()?;
    let canon = canonical(&kind, &yt_id);
    let mut cmd = crate::tools::command("yt-dlp");
    cmd.args(["--flat-playlist", "-J", "--no-warnings"]);
    if kind == "video" {
        cmd.arg("--no-playlist");
    }
    let out = cmd.arg(&canon).output().context("could not run yt-dlp")?;
    if out.stdout.is_empty() {
        bail!("yt-dlp could not read that link: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no answer"));
    }
    let v: Value = serde_json::from_slice(&out.stdout).context("unreadable answer from yt-dlp")?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let (count, duration) = if v.get("_type").and_then(|x| x.as_str()) == Some("playlist") {
        let es = v.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        let d: f64 = es.iter().filter_map(|e| e.get("duration").and_then(|d| d.as_f64())).sum();
        (es.len() as u32, if d > 0.0 { Some(d) } else { None })
    } else {
        (1, v.get("duration").and_then(|d| d.as_f64()))
    };
    let title = s("title");
    if title.is_empty() {
        bail!("yt-dlp found no title for that link (private or removed video?)");
    }
    let uploader = [s("uploader"), s("channel")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
    Ok(Single {
        id: format!("{}-{}", if kind == "video" { "v" } else { "p" }, yt_id),
        kind,
        yt_id,
        url: canon,
        title,
        uploader,
        duration,
        count,
        added: now(),
        status: "saved".into(),
        ..Default::default()
    })
}

fn video_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            video_files(&p, depth - 1, out);
        } else if p.extension().and_then(|x| x.to_str()).map(|x| VIDEO_EXT.contains(&x.to_ascii_lowercase().as_str())).unwrap_or(false) {
            out.push(p);
        }
    }
}

/// Files already in `dir` for this video (the name carries "[<id>]").
pub fn existing(dir: &Path, item: &Single) -> Vec<PathBuf> {
    let mut v = vec![];
    video_files(dir, 1, &mut v);
    let tag = format!("[{}]", item.yt_id);
    v.retain(|p| item.kind == "video" && p.file_name().map(|n| n.to_string_lossy().contains(&tag)).unwrap_or(false));
    v
}

/// Download one video (or all of a playlist) into `dir`. Returns the verified files.
pub fn download(cfg: &crate::config::Config, item: &Single, dir: &Path, on_event: &mut dyn FnMut(Event), cancel: &AtomicBool) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let started = SystemTime::now() - Duration::from_secs(2);
    let y = &cfg.ytdlp;
    let template = if item.kind == "video" {
        "%(title).150s [%(id)s].%(ext)s"
    } else {
        "%(playlist_title).100s/%(playlist_index)03d - %(title).150s [%(id)s].%(ext)s"
    };
    let mut cmd = crate::tools::command("yt-dlp");
    cmd.arg("--ignore-errors")
        .arg("--no-overwrites")
        .arg("--continue")
        .args(["--color", "no_color"])
        .args(["-P", &dir.to_string_lossy()])
        .args(["-o", template])
        .args(["-f", "bv*+ba/b"])
        .args(["-S", "res,vcodec:h264,acodec:aac"])
        .args(["--merge-output-format", "mp4"])
        .arg("--write-info-json")
        .arg("--no-write-playlist-metafiles")
        .args(["--write-subs", "--write-auto-subs", "--sub-langs", &y.subtitle_langs, "--convert-subs", "vtt", "--embed-subs"])
        .args(["--embed-thumbnail", "--embed-metadata"])
        .args(["--sleep-interval", &y.sleep_min.to_string(), "--max-sleep-interval", &y.sleep_max.to_string()])
        .args(["--retries", "10", "--fragment-retries", "10", "--concurrent-fragments", "4"])
        .args(["--progress", "--newline"])
        .args(["--progress-template", "download:CSPROG|%(info.playlist_index|1)s|%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"])
        .args(&y.extra_args);
    if item.kind == "video" {
        cmd.arg("--no-playlist");
    }
    cmd.arg(&item.url).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    on_event(Event::Line(format!("saving to {}", dir.display())));
    let mut child = cmd.spawn().context("could not start yt-dlp")?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).split(b'\n').flatten() {
                let _ = tx.send(String::from_utf8_lossy(&line).trim_end_matches('\r').to_string());
            }
        }));
    }
    drop(tx);
    let mut cancelled = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                if let Some(ev) = fetcher::parse_progress(&line) {
                    on_event(ev);
                } else if !line.trim().is_empty() {
                    on_event(Event::Line(line));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if cancel.load(Ordering::SeqCst) && !cancelled {
            cancelled = true;
            let _ = child.kill();
            on_event(Event::Line("cancelled: stopping yt-dlp (a partial file is kept and resumes next time)".into()));
        }
    }
    let status = child.wait().context("yt-dlp did not finish")?;
    for r in readers {
        let _ = r.join();
    }
    if cancelled {
        bail!("cancelled");
    }
    if !status.success() {
        on_event(Event::Line(format!("yt-dlp exited with {status}; checking what arrived")));
    }
    let mut found = vec![];
    video_files(dir, 1, &mut found);
    found.retain(|p| {
        let tagged = item.kind != "video" || p.file_name().map(|n| n.to_string_lossy().contains(&format!("[{}]", item.yt_id))).unwrap_or(false);
        tagged && std::fs::metadata(p).and_then(|m| m.modified()).map(|t| t >= started).unwrap_or(false)
    });
    found.sort();
    let mut ok = vec![];
    for (i, p) in found.iter().enumerate() {
        match fetcher::verify_file(p) {
            Ok(note) => {
                on_event(Event::Checked { index: i as u32 + 1, ok: true, message: note.unwrap_or_default() });
                ok.push(p.clone());
            }
            Err(why) => {
                let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                if let Some(parent) = p.parent() {
                    if let Ok(rd) = std::fs::read_dir(parent) {
                        for e in rd.flatten() {
                            if e.file_name().to_string_lossy().starts_with(&stem) {
                                let _ = std::fs::remove_file(e.path());
                            }
                        }
                    }
                }
                on_event(Event::Checked { index: i as u32 + 1, ok: false, message: why });
            }
        }
    }
    if ok.is_empty() {
        bail!("nothing was downloaded (see the log)");
    }
    fetcher::tidy_log(dir, on_event);
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_youtube_links() {
        assert_eq!(parse_url("https://www.youtube.com/watch?v=abc_DEF-123&t=5s").unwrap(), ("video".into(), "abc_DEF-123".into()));
        assert_eq!(parse_url("https://youtu.be/abc_DEF-123?si=xyz").unwrap(), ("video".into(), "abc_DEF-123".into()));
        assert_eq!(parse_url("https://m.youtube.com/shorts/abcDEF12345").unwrap().0, "video");
        assert_eq!(parse_url("https://www.youtube.com/watch?v=abc&list=PL123").unwrap(), ("video".into(), "abc".into()));
        assert_eq!(parse_url("https://www.youtube.com/playlist?list=PLabc-1_2").unwrap(), ("playlist".into(), "PLabc-1_2".into()));
        assert!(parse_url("https://example.com/watch?v=abc").is_err());
        assert!(parse_url("https://notyoutube.com/watch?v=abc").is_err());
        assert!(parse_url("https://youtube.com.evil.com/watch?v=abc").is_err());
        assert!(parse_url("file:///etc/passwd").is_err());
        assert!(parse_url("https://www.youtube.com/watch?v=a;rm").is_err());
        assert!(parse_url("https://www.youtube.com/").is_err());
    }

    #[test]
    fn store_roundtrip_and_existing() {
        let d = std::env::temp_dir().join(format!("cs_singles_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(load(&d).items.is_empty());
        let mut s = Store::default();
        s.items.push(Single { id: "v-abc".into(), kind: "video".into(), yt_id: "abc".into(), ..Default::default() });
        s.last_dest = Some("/x".into());
        save(&d, &s).unwrap();
        let l = load(&d);
        assert_eq!(l.items.len(), 1);
        assert_eq!(l.last_dest.as_deref(), Some("/x"));
        std::fs::write(d.join("Some talk [abc].mp4"), b"x").unwrap();
        std::fs::write(d.join("Other [zzz].mp4"), b"x").unwrap();
        assert_eq!(existing(&d, &l.items[0]).len(), 1);
        std::fs::remove_dir_all(&d).ok();
    }
}
