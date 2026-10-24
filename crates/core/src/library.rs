//! Reading what is actually on a drive. Works on folders made by courseshelf-fetch and on
//! folders you made by hand with yt-dlp (names like `07 - Title.mp4`, no .info.json).

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov"];

#[derive(Debug, Clone, Serialize)]
pub struct Lecture {
    /// Playlist position parsed from the file name prefix (0 if there is none).
    pub index: u32,
    /// Stable id: the YouTube id when .info.json exists, otherwise the file name without extension.
    pub key: String,
    pub title: String,
    /// File name inside the course folder.
    pub file: String,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub vcodec: Option<String>,
    pub size_bytes: u64,
    /// Subtitle sidecar file names (.vtt) in the same folder.
    pub subs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CourseDir {
    /// Path relative to the drive root, e.g. `MIT/Matrix_Calculus_18_S096`.
    pub course: String,
    pub lectures: Vec<Lecture>,
}

fn is_fragment(name: &str) -> bool {
    // yt-dlp leaves `Title.f137.mp4` and `Title.mp4.part` while downloading or if a merge failed.
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".part") || lower.ends_with(".ytdl") || lower.ends_with(".temp.mp4") {
        return true;
    }
    if let Some(stem) = Path::new(name).file_stem().and_then(|s| s.to_str()) {
        if let Some((_, tail)) = stem.rsplit_once(".f") {
            return !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

/// "07 - Some title" -> (7, "Some title"); no prefix -> (0, whole stem).
pub fn split_prefix(stem: &str) -> (u32, String) {
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let rest = &stem[digits.len()..];
        if let Some(t) = rest.strip_prefix(" - ").or_else(|| rest.strip_prefix('-')).or_else(|| rest.strip_prefix(". ")).or_else(|| rest.strip_prefix('_')) {
            if let Ok(n) = digits.parse::<u32>() {
                return (n, t.trim().to_string());
            }
        }
    }
    (0, stem.to_string())
}

#[derive(serde::Deserialize, Default)]
struct Info {
    id: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    vcodec: Option<String>,
}

fn read_info(video: &Path) -> Info {
    let p = video.with_extension("info.json");
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Lectures in one folder (not recursive).
pub fn scan_folder(dir: &Path) -> Vec<Lecture> {
    let mut subs_by_stem: HashMap<String, Vec<String>> = HashMap::new();
    let mut videos: Vec<PathBuf> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || is_fragment(&name) {
            continue;
        }
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "vtt" {
            // "<stem>.<lang>.vtt"
            let stem = name.trim_end_matches(".vtt");
            if let Some((video_stem, _lang)) = stem.rsplit_once('.') {
                subs_by_stem.entry(video_stem.to_string()).or_default().push(name.clone());
            }
        } else if VIDEO_EXT.contains(&ext.as_str()) && e.metadata().map(|m| m.is_file()).unwrap_or(false) {
            videos.push(p);
        }
    }
    let mut out: Vec<Lecture> = videos
        .into_iter()
        .map(|p| {
            let file = p.file_name().unwrap().to_string_lossy().to_string();
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let (index, title_from_name) = split_prefix(&stem);
            let info = read_info(&p);
            let mut subs = subs_by_stem.remove(&stem).unwrap_or_default();
            subs.sort();
            Lecture {
                index,
                key: info.id.clone().unwrap_or_else(|| stem.clone()),
                title: if title_from_name.is_empty() { info.title.unwrap_or(stem) } else { title_from_name },
                file,
                duration: info.duration,
                width: info.width,
                height: info.height,
                vcodec: info.vcodec,
                size_bytes: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                subs,
            }
        })
        .collect();
    out.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.file.cmp(&b.file)));
    out
}

/// Every folder under `root` (up to 3 levels deep) that contains at least one video is a course.
pub fn scan_root(root: &Path) -> Vec<CourseDir> {
    let mut out = Vec::new();
    for e in WalkDir::new(root).min_depth(1).max_depth(3).into_iter().filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.')) {
        let Ok(e) = e else { continue };
        if !e.file_type().is_dir() {
            continue;
        }
        let lectures = scan_folder(e.path());
        if !lectures.is_empty() {
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().to_string();
            out.push(CourseDir { course: rel, lectures });
        }
    }
    out.sort_by(|a, b| a.course.to_lowercase().cmp(&b.course.to_lowercase()));
    out
}

/// A lecture is "fully downloaded" if the video is there and no partial file sits beside it.
pub fn downloaded_indexes(course_dir: &Path) -> std::collections::BTreeSet<u32> {
    scan_folder(course_dir).into_iter().filter(|l| l.index > 0).map(|l| l.index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix() {
        assert_eq!(split_prefix("07 - Rate of change"), (7, "Rate of change".into()));
        assert_eq!(split_prefix("100 - X"), (100, "X".into()));
        assert_eq!(split_prefix("Intro"), (0, "Intro".into()));
        assert_eq!(split_prefix("2024 notes"), (0, "2024 notes".into()));
    }

    #[test]
    fn fragments() {
        assert!(is_fragment("01 - A.f137.mp4"));
        assert!(is_fragment("01 - A.mp4.part"));
        assert!(!is_fragment("01 - A.mp4"));
        assert!(!is_fragment("01 - Lecture f2.mp4"));
    }
}
