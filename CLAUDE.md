# CLAUDE.md — CourseShelf

Standing instructions for working on this folder: the PROGRAM of CourseShelf (a public repository, https://github.com/varunjha089/Course-Shelf).
Read this first, then `README.md` (use). If a `CLAUDE.local.md` exists next to this file, read it too: it describes the owner's own setup (data folder,
drives, history) and is never committed. This file explains how the app is built and how to change it safely.

## What it is

An offline Mac app for video courses and PDFs. Rust core + Tauri 2 window + React/TypeScript UI. Tabs: Courses (watch, ticks, resume),
Downloads (yt-dlp downloads, single YouTube links, point/match/move courses, drives), Papers (read-only view of a paper list CSV, `[papers] list`, arXiv
download to `papers/inbox/`), Books (PDF viewer with annotations). A command-line downloader, `courseshelf-fetch`, shares the same core.

**Program and data are separate.** This folder holds only code. Everything of the user's lives in the DATA folder (`config::default_home()`):
`$COURSESHELF_HOME`, else the folder named in `<settings>/home.txt`, else the settings folder itself (`config::settings_dir()`: macOS
`~/Library/Application Support/CourseShelf`). First start writes a starter `config.toml` and an empty `courses.csv` there (`config::ensure_home`, never overwrites).

## Standing rules (from the user; they override convenience)

- **Never start a real download, and never run `courseshelf-fetch run`, unless the user clicks Download or asks.** Metadata lookups (`--flat-playlist -J`) are fine.
- **Never write the paper library** (`[papers] library` / `list`): the Papers tab is read-only. Downloaded papers go to the data folder's `papers/inbox/` only.
- **Never put user data, personal paths or names in the code or in git** (the repository is public): no `/Users/...`, no disk names, no course lists. Examples go in `examples/`.
- **The user's data folder is precious** (PDF annotations, joined books): tests use a scratch data folder; never call write endpoints, delete or rewrite files there.
- **Never delete a user's videos.** Moves copy, verify, then delete; "Remove link" forgets a link only. Ask before any delete of media.
- **git is the history**: commit after every checked change (what and why; attribution lines of the session at the end). Push only when asked; never rewrite
  pushed history. Say in the commit or in `CLAUDE.local.md` what was and was NOT checked.
- The user wears spectacles: keep text large and high-contrast; do not shrink fonts to fit.
- The machine is macOS (BSD `sed`, no `timeout`): edit files with python or the Edit tool, not `sed -i`.

## Layout

```
Cargo.toml                 workspace: crates/core, crates/server, crates/fetch, app/src-tauri
crates/core/src/           no UI, no HTTP: everything that touches disk or tools
  config.rs                config.toml (drives, [mac], [ytdlp], [papers], [pdf_sources]); settings_dir/default_home/ensure_home (data folder,
                           starter config); in-place edit of a drive path
  spec.rs, items.rs        courses.csv rows and the `items` syntax (all, 1-5,8, all-2,15)
  library.rs               folder scan: "NNN - Title.mp4" lectures, partial files, scan_root (courses = folders with videos, 3 levels)
  fetcher.rs               yt-dlp: playlist listing (cached 6 h in cache/), download_course, verify_file (ffprobe length+codec)
  singles.rs               saved single YouTube links (singles.json), lookup, download
  adopt.rs, mover.rs       match hand-downloaded files to playlist titles; verified copy-then-delete move between drives
  locations.rs             locations.json: a course that lives outside <drive>/<course>
  progress.rs              SQLite watch progress in <drive>/.courseshelf/progress.sqlite
  papers.rs                paper list reader (columns S. No, Title, Link, Local Link, ...), arXiv inbox download checks
  tools.rs                 finds yt-dlp/ffmpeg/ffprobe/deno/pdfinfo (see "Finder PATH" below); ALWAYS launch tools via tools::command
crates/server/src/
  lib.rs                   axum server on 127.0.0.1 (random port), cookie token, Host check, routes, media streaming with Range, /api/open
  manage.rs                Downloads-tab endpoints (plan, download, jobs, adopt, move, drives, browse, singles)
  jobs.rs                  one-at-a-time job queue with live progress (kind: download | move | single)
  annot.rs                 PDF annotations (annotations/<key>.json), books list, bookmarks
  collections.rs           book collections: join papers with pdfunite (job kind "collection"), download missing ones, rename/delete
  main.rs                  `courseshelf-serve`: the server alone, for a browser or tests
crates/fetch/              `courseshelf-fetch` CLI (doctor, home, list, plan, run, prune, import-plans)
web/                       React + Vite + TypeScript UI. src/App.tsx (all pages), api.ts (typed calls), pdfview.tsx, styles.css
app/src-tauri/             Tauri shell: tauri.conf.json (productName CourseShelf, beforeBuildCommand builds web/), starts the server in-process
examples/                  config.toml, courses.csv, course_plan.md (what a user starts from)
(no user data here: it is all in the data folder, see "Data files")
```

## How it is built (and how the pieces fit)

1. **Core first.** Anything that touches files, yt-dlp or SQLite lives in `crates/core` with unit tests. Server and CLI only call it.
2. **Server.** The Tauri window loads `http://127.0.0.1:<random port>/` served by `crates/server`. Every `/api` and `/media` request needs a per-run random
   token (cookie), a matching Host header, and paths are joined safely (`safe_join`) so nothing outside a drive/library root is served. Video streams with
   HTTP Range (seeking works). The UI is embedded from `web/dist` at compile time (`crates/server/build.rs` reruns when `web/dist` changes), or served from a
   folder with `courseshelf-serve --ui web/dist` for development.
3. **UI.** One React app, hash routes (`#/manage`, `#/papers`, ...). It never stores state in browser storage (the port changes each launch, so it would be lost):
   settings go to `ui-settings.json` through `/api/settings`. Calls are typed in `web/src/api.ts`.
4. **Jobs.** Downloads/moves run on a single worker thread (`jobs.rs`); the UI polls `/api/manage/jobs` (1 s while active). Cancel kills yt-dlp; partial files resume.
5. **Package.** Tauri wraps it: `app/src-tauri` (identifier `io.github.courseshelf`). The app calls `ensure_home` before starting the server.

## Build, test, install (the commands that work)

```sh
. "$HOME/.cargo/env"                       # cargo is not on PATH in a fresh shell
cd <this folder>                            # quote it: the parent folder name may contain a space
cargo test                                  # all unit tests (core + server)
cd web && npx tsc --noEmit && npm run build && cd ..     # typecheck + build the UI into web/dist
cargo build -p courseshelf-server           # quick compile check of the Rust side
cd app && npx tauri build                   # full app: runs the web build, then bundles
# install: ONE copy only, or Launchpad shows two CourseShelf icons
rm -rf /Applications/CourseShelf.app && cp -R ../target/release/bundle/macos/CourseShelf.app /Applications/ \
  && rm -rf ../target/release/bundle/macos/CourseShelf.app
```
Quit the running app (⌘Q) before replacing it, and tell the user to reopen it. Never leave a second `CourseShelf.app` anywhere (the build output counts).

### Test without touching real data or YouTube
Make a scratch home (`config.toml` with a temp `[drives.local]`, an empty `courses.csv`), put a stand-in `yt-dlp` script first on PATH (answers `-J` with JSON; on
download writes a tiny mp4 with `ffmpeg -f lavfi` plus `.info.json`), then run
`COURSESHELF_HOME=<scratch>/home PATH=<scratch>/bin:/usr/bin:/bin ./target/debug/courseshelf-serve --ui web/dist`. It prints a URL with `?t=<token>`:
`curl -c jar` it once, then call the API with `-b jar`. Headless Chrome (`--headless=new --virtual-time-budget=6000 --dump-dom <url>#/manage`) checks that a page renders.
Use the scratchpad dir for these files, never the real data folder. (`courseshelf-serve` with no `COURSESHELF_HOME` uses the user's REAL data folder via home.txt:
in tests always set `COURSESHELF_HOME` to a scratch folder, or `HOME` to a scratch folder for a true first start.)

## Things that went wrong before (do not repeat)

- **"yt-dlp was not found" from Finder.** A Finder-launched app gets launchd's bare PATH. `tools.rs` searches PATH, then `/opt/homebrew/bin`, `/usr/local/bin`,
  `~/.local/bin`, `~/.cargo/bin`, and gives children the extended PATH. Always use `tools::command`. `open` from a terminal does NOT reproduce a Finder launch;
  test with `env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin <app executable>`. `COURSESHELF_URL_FILE` makes the app write its address to a file (test hook).
- **Settings lost on every restart** when kept in browser storage (random port). Keep them server-side.
- **Same-disk move left the original behind; progress went to the wrong DB on a different drive.** Fixed in `mover.rs` / `progress.rs`; keep the unit tests.
- **Two app copies** in Launchpad (a stale `/Applications` copy plus `target/release/bundle`). See install above.
- **A saved single video does not appear under Courses until it is inside a drive.** A course is a FOLDER (up to 3 levels under a drive root) that contains videos;
  videos directly in a drive root are not a course. The Courses page also shows "Saved video links" for links not downloaded or saved elsewhere.
- yt-dlp quirks: YouTube sometimes answers 429 for subtitles (the lecture is kept, no caption file); `--embed-subs` needs ffmpeg; VP9/AV1 may not play in the app's
  video engine, so H.264 is preferred only as a tie-breaker at equal resolution (`-S "res,vcodec:h264,acodec:aac"`).
- BSD `sed -i` fails without an extension; `timeout` does not exist; use python for scripted edits and assert that each replaced string matched.

## How to add a feature (the pattern used for every tab)

1. Look at `git status`. 2. Logic + unit tests in `crates/core`. 3. Endpoint in `crates/server/src/manage.rs` (or `lib.rs`), route in `lib.rs` (`.route(...)`),
long work as a job via `app.jobs.submit(JobInfo{kind,..}, Box::new(move |ctx| ...))` using `ctx.update/ctx.log` and honouring `ctx.cancel`.
4. Typed call in `web/src/api.ts`, UI in `web/src/App.tsx` (reuse `FolderPicker`, `JobCard`, `.panel/.controls/.btn` classes). If a new job `kind` is added, extend
`Job["kind"]` and `JobCard`. 5. `cargo test`, `npx tsc --noEmit`, `npm run build`, test through the stand-in setup above. 6. `npx tauri build`, install, update
`README.md`, commit. 7. Report what was checked and what was NOT (a real YouTube download, a real external drive, the packaged window's rendering are the usual gaps).

## Data files (all in the data folder, next to config.toml)

| File | Owner | Notes |
|---|---|---|
| `courses.csv` | the user | the app never rewrites it (Start download works even for enabled=no rows) |
| `config.toml` | the user | the app edits only a drive's `path` line, and re-validates before replacing the file |
| `locations.json` | app | course -> folder when not at `<drive>/<course>` |
| `singles.json` | app | saved single-video links + status + last folder |
| `ui-settings.json` | app | font size, theme, autoplay-next |
| `collections.json`, `collections/<id>.pdf` | app | Books made from chosen papers (title, per-paper start page, joined PDF); marks in `annotations/<id>.json` |
| `bookmarks.json` | app | Books-tab bookmarks (list of full PDF paths) |
| `<drive>/.courseshelf/progress.sqlite` | app | travels with the disk |
| `annotations/`, `papers/inbox/`, `cache/` | app | PDF notes, arXiv downloads, playlist listings |

An unplugged disk shows "not connected"; never create folders on a missing disk (the folder browser refuses when the parent does not exist).

## Git
This folder is the repository (branch `main`, remote `origin` = https://github.com/varunjha089/Course-Shelf, public, MIT). Tracked: source (crates/, app/,
web/src, web/public), README, CLAUDE.md, examples/. Not tracked (.gitignore): build output, node_modules, and `*.local.md` (the owner's notes).
A fresh clone must run `cd web && npm install && npm run build` before `cargo build` (the server embeds web/dist).
