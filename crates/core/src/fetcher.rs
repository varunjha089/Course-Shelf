//! Everything that talks to yt-dlp / ffprobe. Shared by the command-line tool and the app.

use anyhow::{bail, Context, Result};
use crate::{
    config::Config,
    items::to_ytdlp,
    library,
    spec::CourseRow,
};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::{AtomicBool, Ordering}, mpsc},
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone)]
pub struct Entry {
    pub index: u32,
    pub title: String,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub title: String,
    pub entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct RawPl {
    title: Option<String>,
    entries: Option<Vec<RawEntry>>,
}

#[derive(Deserialize)]
struct RawEntry {
    title: Option<String>,
    duration: Option<f64>,
}

pub fn fmt_dur(d: Option<f64>) -> String {
    match d {
        None => "-".into(),
        Some(s) => {
            let t = s.round() as u64;
            if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
        }
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn tool_version_line(tool: &str) -> Option<String> {
    let out = crate::tools::command(tool).arg("--version").output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next().map(|l| l.trim().to_string())
}

pub fn require_tools() -> Result<()> {
    for t in ["yt-dlp", "ffmpeg", "ffprobe"] {
        if crate::tools::find(t).is_none() {
            if cfg!(windows) {
                bail!("{t} was not found (looked in PATH and the winget, Scoop and Chocolatey folders); install it, e.g. `winget install yt-dlp.yt-dlp Gyan.FFmpeg DenoLand.Deno`");
            }
            bail!("{t} was not found (looked in PATH and /opt/homebrew/bin, /usr/local/bin, ~/.local/bin); install it, e.g. `brew install {t}`");
        }
    }
    Ok(())
}

/// Playlist listing via `yt-dlp --flat-playlist` (metadata only, no video is fetched). Cached for 6 hours.
pub fn playlist(home: &Path, url: &str, refresh: bool) -> Result<Playlist> {
    let cache_dir = home.join("cache");
    let cache = cache_dir.join(format!("{:016x}.json", fnv(url)));
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .map(|t| SystemTime::now().duration_since(t).unwrap_or(Duration::MAX) < Duration::from_secs(6 * 3600))
        .unwrap_or(false);
    let text = if fresh && !refresh {
        std::fs::read_to_string(&cache)?
    } else {
        let out = crate::tools::command("yt-dlp")
            .args(["--flat-playlist", "-J", "--no-warnings", "--ignore-errors", url])
            .output()
            .context("could not run yt-dlp")?;
        if out.stdout.is_empty() {
            bail!("yt-dlp returned nothing for {url}: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
        }
        let t = String::from_utf8_lossy(&out.stdout).to_string();
        std::fs::create_dir_all(&cache_dir)?;
        std::fs::write(&cache, &t)?;
        t
    };
    let raw: RawPl = serde_json::from_str(&text).context("unreadable playlist listing")?;
    let entries = raw
        .entries
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, e)| Entry { index: i as u32 + 1, title: e.title.unwrap_or_default(), duration: e.duration })
        .collect();
    Ok(Playlist { title: raw.title.unwrap_or_default(), entries })
}

/// The playlist of a course, in lecture order: for an `order = reverse` row the last video is lecture 1.
pub fn course_playlist(home: &Path, row: &CourseRow, refresh: bool) -> Result<Playlist> {
    let mut pl = playlist(home, &row.playlist_url, refresh)?;
    if row.reverse {
        pl.entries.reverse();
        for (i, e) in pl.entries.iter_mut().enumerate() {
            e.index = i as u32 + 1;
        }
    }
    Ok(pl)
}

/// `--playlist-items` for lecture numbers: lecture N of a reversed course is the N-th video from the end (`-N` to yt-dlp).
fn playlist_items(row: &CourseRow, lectures: &[u32]) -> String {
    if row.reverse {
        lectures.iter().map(|n| format!("-{n}")).collect::<Vec<_>>().join(",")
    } else {
        to_ytdlp(lectures)
    }
}

#[derive(Debug)]
pub struct CourseResult {
    pub course: String,
    pub drive: String,
    pub wanted: usize,
    pub downloaded: usize,
    pub failed: Vec<String>,
    pub warnings: Vec<String>,
}

impl CourseResult {
    pub fn ok(row: &CourseRow, wanted: usize, downloaded: usize) -> CourseResult {
        CourseResult { course: row.course.clone(), drive: row.drive.clone(), wanted, downloaded, failed: vec![], warnings: vec![] }
    }
    pub fn summary(&self) -> String {
        let mut s = format!("{}: {} downloaded and verified, {} failed", self.course, self.downloaded, self.failed.len());
        for f in &self.failed {
            s.push_str(&format!("\n   FAILED {f}"));
        }
        for w in &self.warnings {
            s.push_str(&format!("\n   note   {w}"));
        }
        s
    }
}

#[derive(Deserialize)]
struct Probe {
    streams: Option<Vec<PStream>>,
    format: Option<PFormat>,
}
#[derive(Deserialize)]
struct PStream {
    codec_name: Option<String>,
    height: Option<u32>,
}
#[derive(Deserialize)]
struct PFormat {
    duration: Option<String>,
}

#[derive(Deserialize)]
struct InfoDur {
    duration: Option<f64>,
}

/// Check one finished file. Returns Err(reason) if it is unusable, Ok(Some(note)) for a playable-but-notable file.
pub fn verify_file(video: &Path) -> std::result::Result<Option<String>, String> {
    let out = crate::tools::command("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,height:format=duration", "-of", "json"])
        .arg(video)
        .output()
        .map_err(|e| format!("ffprobe failed: {e}"))?;
    let p: Probe = serde_json::from_slice(&out.stdout).map_err(|_| "ffprobe could not read the file".to_string())?;
    let st = p.streams.and_then(|s| s.into_iter().next()).ok_or("no video stream")?;
    let dur: f64 = p.format.and_then(|f| f.duration).and_then(|d| d.parse().ok()).ok_or("no duration")?;
    if let Some(want) = std::fs::read_to_string(video.with_extension("info.json")).ok().and_then(|s| serde_json::from_str::<InfoDur>(&s).ok()).and_then(|i| i.duration) {
        if (dur - want).abs() > (want * 0.01).max(3.0) {
            return Err(format!("length {dur:.0}s but YouTube says {want:.0}s (incomplete download?)"));
        }
    }
    let codec = st.codec_name.unwrap_or_default();
    if codec != "h264" {
        return Ok(Some(format!("{} is {codec} {}p: may not play in the app's video engine", video.file_name().unwrap().to_string_lossy(), st.height.unwrap_or(0))));
    }
    Ok(None)
}

/// What a running download reports back, line by line.
#[derive(Debug, Clone)]
pub enum Event {
    /// A raw yt-dlp output line (already stripped of our own progress markers).
    Line(String),
    /// A lecture started downloading.
    Item { index: u32, title: String },
    Progress { index: u32, percent: f32, speed: String, eta: String },
    /// A finished lecture passed (or failed) the length/codec check.
    Checked { index: u32, ok: bool, message: String },
}

pub fn parse_progress(line: &str) -> Option<Event> {
    // CSPROG|<playlist_index>|<percent>|<speed>|<eta>
    let rest = line.strip_prefix("CSPROG|")?;
    let mut it = rest.split('|');
    let index: u32 = it.next()?.trim().parse().ok()?;
    let percent: f32 = it.next()?.trim().trim_end_matches('%').trim().parse().unwrap_or(0.0);
    let speed = it.next().unwrap_or("").trim().to_string();
    let eta = it.next().unwrap_or("").trim().to_string();
    Some(Event::Progress { index, percent, speed, eta })
}

/// Download the listed playlist positions into `dir`. `on_event` is called for every output line and
/// progress tick; setting `cancel` stops yt-dlp (partial files are kept and resume next time).
pub fn download_course(
    cfg: &Config,
    row: &CourseRow,
    missing: &[u32],
    dir: &Path,
    on_event: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> Result<CourseResult> {
    std::fs::create_dir_all(dir)?;
    let before: BTreeSet<u32> = library::downloaded_indexes(dir);
    let sort = match row.quality_cap {
        Some(c) => format!("res:{c},vcodec:h264,acodec:aac"),
        None => "res,vcodec:h264,acodec:aac".to_string(),
    };
    let y = &cfg.ytdlp;
    // files start with the lecture number; for a reversed playlist that is counted from the end
    let (number, progress_number) = if row.reverse {
        ("playlist_count+1-playlist_index", "info.playlist_count+1-info.playlist_index")
    } else {
        ("playlist_index", "info.playlist_index")
    };
    let mut cmd = crate::tools::command("yt-dlp");
    cmd.arg("--ignore-errors")
        .arg("--no-overwrites")
        .arg("--continue")
        .args(["--color", "no_color"])
        .args(["-P", &dir.to_string_lossy()])
        .args(["-o", &format!("%({number})03d - %(title)s.%(ext)s")])
        .args(["-f", "bv*+ba/b"])
        .args(["-S", &sort])
        .args(["--merge-output-format", "mp4"])
        .arg("--write-info-json")
        .arg("--no-write-playlist-metafiles")
        .args(["--write-subs", "--write-auto-subs", "--sub-langs", &y.subtitle_langs, "--convert-subs", "vtt", "--embed-subs"])
        .args(["--embed-thumbnail", "--embed-metadata"])
        .args(["--sleep-interval", &y.sleep_min.to_string(), "--max-sleep-interval", &y.sleep_max.to_string()])
        .args(["--retries", "10", "--fragment-retries", "10", "--concurrent-fragments", "4"])
        .args(["--playlist-items", &playlist_items(row, missing)])
        .args(["--progress", "--newline"])
        .args(["--progress-template", &format!("download:CSPROG|%({progress_number})s|%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s")])
        .args(&y.extra_args)
        .arg(&row.playlist_url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    on_event(Event::Line(format!("quality: {}", row.quality_cap.map(|c| format!("up to {c}p")).unwrap_or_else(|| "best available".into()))));
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
                if let Some(ev) = parse_progress(&line) {
                    on_event(ev);
                } else if let Some(p) = line.strip_prefix("[download] Destination: ") {
                    let stem = Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let (index, title) = library::split_prefix(&stem);
                    if index > 0 && !p.ends_with(".vtt") && !p.contains(".f") {
                        on_event(Event::Item { index, title });
                    }
                    on_event(Event::Line(line));
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
            on_event(Event::Line("cancelled: stopping yt-dlp (partial files are kept and will resume)".into()));
        }
    }
    let status = child.wait().context("yt-dlp did not finish")?;
    for r in readers {
        let _ = r.join();
    }
    if cancelled {
        return Ok(CourseResult { course: row.course.clone(), drive: row.drive.clone(), wanted: missing.len(), downloaded: 0, failed: vec!["cancelled".into()], warnings: vec![] });
    }
    if !status.success() {
        on_event(Event::Line(format!("yt-dlp exited with {status}; checking what arrived")));
    }

    let mut res = CourseResult { course: row.course.clone(), drive: row.drive.clone(), wanted: missing.len(), downloaded: 0, failed: vec![], warnings: vec![] };
    let now = library::scan_folder(dir);
    for &i in missing {
        let Some(l) = now.iter().find(|l| l.index == i) else {
            res.failed.push(format!("{i:03}: not downloaded"));
            on_event(Event::Checked { index: i, ok: false, message: "not downloaded".into() });
            continue;
        };
        if before.contains(&i) {
            continue;
        }
        let path: PathBuf = dir.join(&l.file);
        match verify_file(&path) {
            Ok(note) => {
                res.downloaded += 1;
                on_event(Event::Checked { index: i, ok: true, message: note.clone().unwrap_or_default() });
                if let Some(n) = note {
                    res.warnings.push(n);
                }
            }
            Err(why) => {
                // Remove only this run's bad file (and its sidecars) so the next run fetches it again.
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                if let Ok(rd) = std::fs::read_dir(dir) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy().starts_with(&stem) {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                on_event(Event::Checked { index: i, ok: false, message: why.clone() });
                res.failed.push(format!("{i:03} {}: {why}", l.title));
            }
        }
    }
    tidy_log(dir, on_event);
    Ok(res)
}

/// Shrinks and hides the side files left beside the videos (see `tidy`), and says so in the log.
pub fn tidy_log(dir: &Path, on_event: &mut dyn FnMut(Event)) {
    match crate::tidy::tidy(dir) {
        Ok(r) if r.shrunk > 0 || r.hidden > 0 => on_event(Event::Line(format!(
            "tidied the folder: {} details file(s) shrunk ({:.1} MB saved), {} side file(s) hidden",
            r.shrunk,
            r.bytes_saved as f64 / 1e6,
            r.hidden
        ))),
        Ok(_) => {}
        Err(e) => on_event(Event::Line(format!("could not tidy the folder: {e:#}"))),
    }
}

pub fn write_status(cfg: &Config, results: &[CourseResult]) -> Result<()> {
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for d in &cfg.drives {
        let mine: Vec<&CourseResult> = results.iter().filter(|r| r.drive == d.name).collect();
        if mine.is_empty() || !d.connected() {
            continue;
        }
        d.init()?;
        let mut w = csv::Writer::from_path(d.state_dir().join("status.csv"))?;
        w.write_record(["course", "wanted_this_run", "downloaded_now", "failed", "notes", "run_unix"])?;
        for r in mine {
            w.write_record([&r.course, &r.wanted.to_string(), &r.downloaded.to_string(), &r.failed.len().to_string(), &r.warnings.join(" | "), &now.to_string()])?;
        }
        w.flush()?;
    }
    Ok(())
}

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() >= 3).map(|t| t.to_string()).collect()
}

/// Compare the plan's titles (plans/<course>.tsv, if present) with the live playlist for the chosen items.
pub fn verify_titles(home: &Path, row: &CourseRow, pl: &Playlist) -> Vec<String> {
    let path = home.join("plans").join(format!("{}.tsv", row.course.replace('/', "__")));
    let Ok(txt) = std::fs::read_to_string(path) else { return vec![] };
    let mut warnings = vec![];
    for line in txt.lines() {
        let Some((n, title)) = line.split_once('\t') else { continue };
        let Ok(n) = n.parse::<u32>() else { continue };
        if !row.items.resolve(pl.entries.len() as u32).contains(&n) {
            continue;
        }
        let want = tokens(title);
        if want.len() < 3 {
            continue; // too little to compare, e.g. "Lec 5"
        }
        let live = tokens(&pl.entries[n as usize - 1].title);
        let hit = want.iter().filter(|t| live.contains(t)).count();
        if (hit as f64) / (want.len() as f64) < 0.5 {
            warnings.push(format!("#{n}: plan says \"{title}\" but playlist position {n} is \"{}\"", pl.entries[n as usize - 1].title));
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_courses_count_lectures_from_the_end() {
        let mut row = CourseRow {
            course: "P/C".into(),
            playlist_url: String::new(),
            items: crate::items::ItemSpec::parse("all").unwrap(),
            items_text: "all".into(),
            priority: 1,
            drive: "local".into(),
            quality_cap: None,
            enabled: true,
            note: String::new(),
            reverse: false,
            line: 2,
        };
        assert_eq!(playlist_items(&row, &[1, 2, 3, 7]), "1-3,7");
        row.reverse = true;
        assert_eq!(playlist_items(&row, &[1, 2, 7]), "-1,-2,-7");

        // the cached listing (newest first) comes back in lecture order
        let home = std::env::temp_dir().join(format!("cs_rev_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("cache")).unwrap();
        row.playlist_url = "https://www.youtube.com/playlist?list=PLrev".into();
        let json = r#"{"title": "T", "entries": [{"title": "Lec 3"}, {"title": "Lec 2"}, {"title": "Lec 1"}]}"#;
        std::fs::write(home.join("cache").join(format!("{:016x}.json", fnv(&row.playlist_url))), json).unwrap();
        let pl = course_playlist(&home, &row, false).unwrap();
        let got: Vec<(u32, &str)> = pl.entries.iter().map(|e| (e.index, e.title.as_str())).collect();
        assert_eq!(got, [(1, "Lec 1"), (2, "Lec 2"), (3, "Lec 3")]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
