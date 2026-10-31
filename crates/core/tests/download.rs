//! A course download from start to finish, with a stand-in `yt-dlp` (a shell script that makes tiny real videos with ffmpeg and
//! writes the side files YouTube would give), so no network is used. It runs the real `fetcher::download_course`: the yt-dlp
//! call, the length/codec check with ffprobe, and the tidy step. A separate test binary, because it puts the stand-in first
//! on PATH. Skipped where ffmpeg is not installed.

#![cfg(unix)]

use courseshelf_core::{config::Config, fetcher, library, spec, tools};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Answers a download call like yt-dlp: for each position in --playlist-items, a 2 s H.264 video "NNN - Lecture N.mp4" in the
/// -P folder, its .info.json (with the bulky caption list real ones have) and an English .vtt.
const STAND_IN: &str = r#"#!/bin/sh
dir=""; items=""
while [ $# -gt 0 ]; do
  case "$1" in
    -P) dir="$2"; shift ;;
    --playlist-items) items="$2"; shift ;;
  esac
  shift
done
for part in $(echo "$items" | tr ',' ' '); do
  case "$part" in
    *-*) list=$(seq "${part%-*}" "${part#*-}") ;;
    *) list="$part" ;;
  esac
  for i in $list; do
    stem="$dir/$(printf %03d "$i") - Lecture $i"
    echo "[download] Destination: $stem.mp4"
    ffmpeg -loglevel error -y -f lavfi -i testsrc=duration=2:size=160x120:rate=10 -c:v libx264 -pix_fmt yuv420p "$stem.mp4" || exit 1
    printf '{"id": "vid%s", "title": "Lecture %s", "duration": 2, "automatic_captions": {"xx": "%s"}}' "$i" "$i" "$(head -c 6000 /dev/zero | tr '\0' 'c')" > "$stem.info.json"
    printf 'WEBVTT\n\n00:00.000 --> 00:01.000\nHello\n' > "$stem.en.vtt"
  done
done
"#;

#[test]
fn download_is_verified_and_tidied() {
    if tools::find("ffmpeg").is_none() || tools::find("ffprobe").is_none() {
        eprintln!("ffmpeg/ffprobe not installed; skipping");
        return;
    }
    let base = std::env::temp_dir().join(format!("cs_dl_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (bin, home, drive) = (base.join("bin"), base.join("home"), base.join("drive"));
    for d in [&bin, &home, &drive] {
        std::fs::create_dir_all(d).unwrap();
    }
    let script = bin.join("yt-dlp");
    std::fs::write(&script, STAND_IN).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    // SAFETY: this test binary has a single test, so no other thread reads the environment meanwhile.
    unsafe { std::env::set_var("PATH", path) };
    assert_eq!(tools::find("yt-dlp").as_deref(), Some(script.as_path()), "the stand-in comes first");

    std::fs::write(home.join("config.toml"), format!("courses_csv = \"courses.csv\"\n[drives.local]\npath = {:?}\n", drive.to_string_lossy())).unwrap();
    std::fs::write(
        home.join("courses.csv"),
        "course,playlist_url,items,priority,drive,quality_cap,enabled,note\nP/Course,https://www.youtube.com/playlist?list=PLstandin,all,1,local,,yes,\n",
    )
    .unwrap();
    let cfg = Config::load(&home).unwrap();
    let row = spec::load_courses(&cfg.courses_csv).unwrap().remove(0);
    let dir = drive.join(&row.course);

    let mut lines = Vec::new();
    let res = fetcher::download_course(&cfg, &row, &[1, 2], &dir, &mut |e| lines.push(format!("{e:?}")), &AtomicBool::new(false)).unwrap();
    assert_eq!((res.downloaded, res.failed.len()), (2, 0), "{res:?}\n{}", lines.join("\n"));

    // the app reads both lectures, keyed by their YouTube id, with subtitles
    let lectures = library::scan_folder(&dir);
    let keys: Vec<&str> = lectures.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(keys, ["vid1", "vid2"]);
    assert_eq!(lectures[0].subs, ["001 - Lecture 1.en.vtt"]);
    // and the folder was tidied: the details files are small now
    let info = std::fs::read_to_string(dir.join("001 - Lecture 1.info.json")).unwrap();
    assert!(info.len() < 300 && !info.contains("automatic_captions"), "{info}");
    assert!(lines.iter().any(|l| l.contains("tidied the folder")), "{}", lines.join("\n"));
    assert!(Path::new(&dir.join("002 - Lecture 2.mp4")).is_file());
    let _ = std::fs::remove_dir_all(&base);
}
