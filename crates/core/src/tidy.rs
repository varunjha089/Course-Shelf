//! Tidying a course folder after a download: yt-dlp leaves a `.info.json` (often several MB, mostly the list of automatic
//! caption languages) and `.vtt` subtitles beside each video. Both are still needed (the YouTube id in `.info.json` keys
//! the watch progress; the player shows the `.vtt`), so they stay where they are, under the same names. Tidying
//! - shrinks each `.info.json` to the fields CourseShelf reads plus a few that describe the lecture, and
//! - marks the side files hidden (macOS: the Finder "hidden" flag), so the folder shows only the videos.
//!
//! Nothing else is touched: videos, partial downloads and files that do not belong to a video stay as they are.

use anyhow::Result;
use std::path::Path;

use crate::library;

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
    /// Side files marked hidden (0 where the system has no hidden flag).
    pub hidden: usize,
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

/// Sets the Finder's hidden flag. Returns whether the file is hidden now.
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
        return true;
    }
    // SAFETY: as above; chflags only changes the file's flags.
    unsafe { libc::chflags(c.as_ptr(), st.st_flags | libc::UF_HIDDEN) == 0 }
}

/// Other systems have no hidden flag that the file managers honour for ordinary names; the files stay visible.
#[cfg(not(target_os = "macos"))]
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
        if cfg!(target_os = "macos") {
            assert_eq!(r.hidden, 2, "the .info.json and the .vtt");
        }
        assert_eq!(std::fs::read_to_string(d.join("notes.txt")).unwrap(), "mine");
        assert!(d.join("002 - Next.mp4.part").is_file());

        // running again changes nothing
        let again = tidy(&d).unwrap();
        assert_eq!((again.shrunk, again.bytes_saved), (0, 0));
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
