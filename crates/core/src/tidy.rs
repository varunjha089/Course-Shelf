//! Tidying a course folder after a download: yt-dlp leaves a `.info.json` (often several MB, mostly the list of automatic
//! caption languages) and `.vtt` subtitles beside each video. Both are still needed (the YouTube id in `.info.json` keys
//! the watch progress; the player shows the `.vtt`), so they stay where they are, under the same names. Tidying
//! - shrinks each `.info.json` to the fields CourseShelf reads plus a few that describe the lecture, and
//! - marks the side files hidden (the Finder hidden flag on macOS, the hidden attribute on Windows), so the folder shows only the videos.
//!
//! Nothing else is touched: videos, partial downloads and files that do not belong to a video stay as they are.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::{library, locations};

/// Fields kept in a shrunk `.info.json`. `id`, `title`, `duration`, `width`, `height` and `vcodec` are read by
/// `library::scan_folder` and `fetcher::verify_file`; the rest describe the lecture for a person reading the file.
const KEEP: &[&str] = &[
    "id",
    "title",
    "duration",
    "width",
    "height",
    "vcodec",
    "acodec",
    "fps",
    "webpage_url",
    "channel",
    "uploader",
    "upload_date",
    "playlist_title",
    "playlist_index",
    "description",
];

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// `.info.json` files shrunk.
    pub shrunk: usize,
    /// Bytes saved by shrinking.
    pub bytes_saved: u64,
    /// Side files newly marked hidden (0 where the system has no hidden flag, or when they already were).
    pub hidden: usize,
}

impl Report {
    pub fn add(&mut self, other: &Report) {
        self.shrunk += other.shrunk;
        self.bytes_saved += other.bytes_saved;
        self.hidden += other.hidden;
    }
}

/// Every course folder that can be reached now: each course's own folder (locations.json) and every folder with videos on a
/// connected drive (up to 3 levels deep), each once. `only_root` limits it to folders inside that drive folder.
pub fn course_folders(cfg: &Config, home: &Path, only_root: Option<&Path>) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = locations::load(home).into_values().filter(|p| p.is_dir()).collect();
    for d in cfg.drives.iter().filter(|d| d.connected() && d.root.is_dir()) {
        all.extend(library::scan_root(&d.root).into_iter().map(|c| d.root.join(c.course)));
    }
    if let Some(root) = only_root {
        all.retain(|p| locations::relative_inside(p, root).is_some());
    }
    let mut seen = std::collections::BTreeSet::new();
    all.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    all
}

/// Tidies the side files of every video in `dir` (not recursive). Safe to run again: a shrunk file stays as it is.
pub fn tidy(dir: &Path) -> Result<Report> {
    let mut report = Report::default();
    let names: Vec<String> = std::fs::read_dir(dir)?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    for lecture in library::scan_folder(dir) {
        let stem = Path::new(&lecture.file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let info = dir.join(format!("{stem}.info.json"));
        if let Some(saved) = if info.is_file() { shrink(&info)? } else { None } {
            report.shrunk += 1;
            report.bytes_saved += saved;
        }
        // "<stem>.info.json" and the subtitles "<stem>.<lang>.vtt"
        let side = names.iter().filter(|n| {
            n.strip_prefix(stem.as_str())
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|rest| rest == "info.json" || (rest.ends_with(".vtt") && !rest.contains('/')))
        });
        for name in side {
            if hide(&dir.join(name)) {
                report.hidden += 1;
            }
        }
    }
    Ok(report)
}

/// Rewrites `path` with only the `KEEP` fields. Returns the bytes saved, or None when there was nothing to drop
/// (already shrunk, or not a JSON object: then the file is left alone).
fn shrink(path: &Path) -> Result<Option<u64>> {
    let text = std::fs::read_to_string(path)?;
    let Ok(serde_json::Value::Object(full)) = serde_json::from_str::<serde_json::Value>(&text) else { return Ok(None) };
    if full.keys().all(|k| KEEP.contains(&k.as_str())) {
        return Ok(None);
    }
    let slim: serde_json::Map<String, serde_json::Value> = full.into_iter().filter(|(k, _)| KEEP.contains(&k.as_str())).collect();
    let new = serde_json::to_string_pretty(&slim)?;
    let tmp = path.with_file_name(format!(".tidy-{}", path.file_name().unwrap().to_string_lossy()));
    std::fs::write(&tmp, &new)?;
    std::fs::rename(&tmp, path)?;
    Ok(Some((text.len() as u64).saturating_sub(new.len() as u64)))
}

/// Sets the Finder's hidden flag. Returns whether it was set now (false if it already was, or could not be).
#[cfg(target_os = "macos")]
fn hide(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a properly sized buffer.
    if unsafe { libc::stat(c.as_ptr(), &mut st) } != 0 {
        return false;
    }
    if st.st_flags & libc::UF_HIDDEN != 0 {
        return false;
    }
    // SAFETY: as above; chflags only changes the file's flags.
    unsafe { libc::chflags(c.as_ptr(), st.st_flags | libc::UF_HIDDEN) == 0 }
}

/// Sets the Windows "hidden" attribute (File Explorer hides such files unless "Hidden items" is on). Returns whether it was set now.
#[cfg(windows)]
fn hide(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, INVALID_FILE_ATTRIBUTES};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives both calls.
    let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    attrs & FILE_ATTRIBUTE_HIDDEN == 0 && unsafe { SetFileAttributesW(wide.as_ptr(), attrs | FILE_ATTRIBUTE_HIDDEN) } != 0
}

/// Linux file managers hide only names that start with a dot, and renaming would break the app's lookups: the files stay visible.
#[cfg(not(any(target_os = "macos", windows)))]
fn hide(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cs_tidy_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn shrinks_and_hides_side_files_only() {
        let d = dir("a");
        std::fs::write(d.join("001 - Intro.mp4"), b"video").unwrap();
        let big = serde_json::json!({"id": "abc123", "title": "Intro", "duration": 61.0, "width": 1920, "height": 1080,
            "vcodec": "avc1", "automatic_captions": {"fr": ["x".repeat(5000)]}, "formats": [1, 2, 3]});
        std::fs::write(d.join("001 - Intro.info.json"), big.to_string()).unwrap();
        std::fs::write(d.join("001 - Intro.en.vtt"), "WEBVTT\n").unwrap();
        std::fs::write(d.join("notes.txt"), "mine").unwrap();
        std::fs::write(d.join("002 - Next.mp4.part"), b"partial").unwrap();

        let before = library::scan_folder(&d);
        let r = tidy(&d).unwrap();
        assert_eq!(r.shrunk, 1);
        assert!(r.bytes_saved > 4000);
        let slim: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join("001 - Intro.info.json")).unwrap()).unwrap();
        assert_eq!(slim["id"], "abc123");
        assert!(slim.get("automatic_captions").is_none() && slim.get("formats").is_none());
        // the app reads the same lecture as before: same key (progress), duration and subtitles
        let after = library::scan_folder(&d);
        assert_eq!((&after[0].key, after[0].duration, &after[0].subs), (&before[0].key, before[0].duration, &before[0].subs));
        assert_eq!(after[0].key, "abc123");
        if cfg!(any(target_os = "macos", windows)) {
            assert_eq!(r.hidden, 2, "the .info.json and the .vtt");
        }
        assert_eq!(std::fs::read_to_string(d.join("notes.txt")).unwrap(), "mine");
        assert!(d.join("002 - Next.mp4.part").is_file());

        // running again changes nothing
        let again = tidy(&d).unwrap();
        assert_eq!(again, Report::default(), "nothing left to do");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn leaves_a_damaged_info_file_alone() {
        let d = dir("b");
        std::fs::write(d.join("001 - A.mp4"), b"video").unwrap();
        std::fs::write(d.join("001 - A.info.json"), "{not json").unwrap();
        assert_eq!(tidy(&d).unwrap().shrunk, 0);
        assert_eq!(std::fs::read_to_string(d.join("001 - A.info.json")).unwrap(), "{not json");
        let _ = std::fs::remove_dir_all(&d);
    }
}
