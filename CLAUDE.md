# CLAUDE.md: CourseShelf

Instructions for working on this folder: the program of CourseShelf, a public repository (https://github.com/varunjha089/Course-Shelf, MIT).
Read this file first, then `README.md` (how it is used). If `CLAUDE.local.md` exists next to this file, read it too: it holds the owner's own setup (his data
folder, drives, history) and is never committed.

## What it is

An offline library for video courses and PDFs. Rust core + Tauri 2 window + React/TypeScript UI, made on macOS; GitHub Actions builds and tests it on macOS,
Windows and Linux (`.github/workflows/build.yml`).
Tabs: **Courses** (watch, resume, ticks, copy to this computer), **Downloads** (yt-dlp downloads, single YouTube links, point / match / move courses, drives),
**Papers** (a paper list CSV: your own library through `[papers]`, else a starter list in the data folder; question banks, arXiv download to the inbox,
"make a book", "add papers"), **Books** (PDF viewer with annotations).
A command-line tool, `courseshelf-fetch` (doctor, home, list, plan, run, prune, tidy, import-plans), shares the same core. App version: `app/src-tauri/tauri.conf.json`.

**Program and data are separate.** This folder holds only code. Everything of a user's lives in the DATA folder (`config::default_home()`):
`$COURSESHELF_HOME`, else the folder written in `<settings>/home.txt`, else the settings folder itself (`config::settings_dir()`: macOS
`~/Library/Application Support/CourseShelf`, Linux `~/.config/courseshelf`, Windows `%APPDATA%\CourseShelf`). The first start writes a starter `config.toml` and an
empty `courses.csv` there (`config::ensure_home`, which never overwrites). `courseshelf-fetch home --set FOLDER` points it at another folder.

## Rules

- **Shareable.** No user data, personal path, disk name, course list or name in the code, the docs or git: the repository is public. Examples go in `examples/`;
  the owner's notes go in `*.local.md` (ignored by git).
- **A user's data folder is precious** (PDF annotations, joined books, bookmarks): never edit, move or delete files in it; tests use a scratch data folder.
- **Never start a real download, and never run `courseshelf-fetch run`, unless the user presses Download or asks.** Metadata lookups (`--flat-playlist -J`) are fine.
- **Never delete a user's videos.** A move copies, verifies, then deletes; "Remove link" only forgets a link. Ask before any delete of media.
- **The paper list is written in ONE place only: "Add papers"** (`core/addpapers.rs`, `POST /api/papers/add`), on the user's click, through the library's own
  `[papers] add_command` when set (it knows the library's rules), else appended with a backup. Nothing else writes the list or any other file of the library;
  downloads go to the data folder's `papers/inbox/` only. A duplicate (same arXiv id, DOI or link) is refused by the server before the job starts.
  Without a `[papers]` section the Papers tab uses the **starter list** `<data folder>/papers/papers.csv` (`PapersPaths::starter`, `starter: true`): it reads as
  empty until the first "Add papers" makes it (`addpapers::start_list`, headings `S. No,Title,DL_Topic,Link,Local Link,Notes`). Only the starter list is ever
  created; a configured library or list that is missing stays an error (server `load_list`). Keep the `Paper_Database.csv` default for `[papers]`: existing
  configs rely on it.
- **Large, high-contrast text** (the owner wears spectacles); do not shrink fonts to fit.
- On macOS (BSD `sed`, no `timeout`) edit files with Python or the Edit tool and assert that each replaced string matched.

## Layout

```
Cargo.toml                 workspace: crates/core, crates/server, crates/fetch, app/src-tauri
crates/core/src/           no UI, no HTTP: everything that touches disk or tools (with unit tests)
  config.rs                config.toml (drives, [mac], [ytdlp], [papers] library/list/banks_dir/inbox, [pdf_sources]); settings_dir, default_home,
                           home_pointer, ensure_home + STARTER_CONFIG; PapersPaths::starter (no [papers]); in-place edit of a drive's path
  spec.rs, items.rs        courses.csv rows and the `items` syntax (all, 1-5,8, all-2,15)
  library.rs               folder scan: "NNN - Title.mp4" lectures, partial files; a course = a folder with videos, up to 3 levels under a drive
  tidy.rs                  after a download / single / move: shrink each .info.json to the KEEP fields and set the macOS hidden flag on the
                           .info.json and .vtt (they stay beside the video, same names: the id keys progress, the .vtt feeds the player).
                           A field the app starts to read from .info.json MUST be added to KEEP, or tidied folders lose it
  fetcher.rs               yt-dlp: playlist listing (cached 6 h in cache/), download_course, verify_file (ffprobe length + codec)
  singles.rs               saved single YouTube links (singles.json), lookup, download
  adopt.rs, mover.rs       match hand-downloaded files to playlist titles; verified copy-then-delete move between drives
  locations.rs             locations.json: a course that lives outside <drive>/<course>. `locate` (the ONE way to find a course's folder; the
                           app and courseshelf-fetch both use it), `host_drive` (the drive that folder is really on); relative_inside names such a folder when it lies inside a
                           drive, so drive scans skip it (it is listed once, under the course's own name, as a copy on that drive)
  progress.rs              SQLite watch progress in <drive>/.courseshelf/progress.sqlite (travels with the disk)
  papers.rs                paper list reader (columns S. No, Title, Link, Local Link, ...), arXiv inbox
  addpapers.rs             "Add papers": parse pasted ids / DOIs, arXiv API + Crossref lookup (curl), append with a backup when there is no add_command;
                           start_list makes the starter list
  tools.rs                 finds yt-dlp / ffmpeg / ffprobe / deno / pdfinfo (also `name.exe` and winget/Scoop/Chocolatey folders on Windows); ALWAYS
                           launch tools through tools::command; tools::open opens or reveals a file (open / explorer / xdg-open), never call `open` directly
crates/server/src/
  lib.rs                   axum server on 127.0.0.1 (random port), cookie token, Host check, routes, video streaming with Range
  manage.rs                Downloads-tab endpoints (overview incl. the data folder, plan, download, jobs, adopt, move, drives, browse, singles).
                           Download places: `dest_root` = a drive's folder (course goes to <root>/<course>), `dest_folder` = exactly that folder
                           (`exact_located`); moves take `to_drive` or `to_folder` the same way. A place other than <csv drive>/<course> is
                           recorded in locations.json (and the course's progress moves to <folder>/.courseshelf); the usual place clears it.
                           The overview's `drive` is where the folder IS (`locations::host_drive`), `planned_drive` the courses.csv one
  jobs.rs                  one-at-a-time job queue with live progress (kinds: download, move, single, collection, addpapers)
  annot.rs                 annotations (annotations/<key>.json, key = hash of the PDF's bytes), books list, bookmarks
  collections.rs           books joined from papers (pdfunite), missing papers downloaded first, rename / reorder / delete
  main.rs                  courseshelf-serve: the server alone, for a browser or tests
crates/fetch/src/          courseshelf-fetch (main.rs) and import-plans (importer.rs: provider = first word of the heading)
web/                       React + Vite + TypeScript: src/App.tsx (all pages, hash router at the top), api.ts (typed calls), annotmodel.ts, styles.css; public/pdfjs
  src/pdfview.tsx          the PDF viewer: toolbar, ONE keydown handler for every key (tools g v p h e r o l a t, [ ] width, +/- zoom, \ side panel),
                           side panel (Contents / Notes / Bookmarks / Search; folds to a strip), Fit width follows the scroller's size (ResizeObserver)
app/src-tauri/             Tauri shell (identifier io.github.courseshelf): calls ensure_home, starts the server in-process, opens the window
examples/                  config.toml, courses.csv, course_plan.md: what a new user starts from
```

## How the pieces fit

1. **Core first.** Anything that touches files, yt-dlp or SQLite lives in `crates/core` with unit tests; the server and the CLI only call it.
2. **Server.** The window loads `http://127.0.0.1:<random port>/`. Every `/api` and `/media` request needs the per-run token (cookie) and a matching Host header;
   paths are joined with `safe_join`, so nothing outside a drive or library root is served. The UI is embedded from `web/dist` at compile time
   (`crates/server/build.rs`), or served from a folder with `courseshelf-serve --ui web/dist` during development.
3. **UI.** One React app with hash routes (`#/manage`, `#/papers`, ...). It never keeps state in browser storage (the port changes every launch): settings go to
   `ui-settings.json` through `/api/settings`.
4. **Jobs.** Downloads and moves run one at a time on a worker thread; the UI polls `/api/manage/jobs`. Cancel stops yt-dlp; partial files resume.

## How to add a feature

1. `git status`. 2. Logic + unit tests in `crates/core`. 3. An endpoint in `crates/server/src/manage.rs` (or `lib.rs`) and its route; long work as a job
(`app.jobs.submit(JobInfo{kind, ..}, Box::new(move |ctx| ...))`, using `ctx.update` / `ctx.log` and honouring `ctx.cancel`). 4. A typed call in `web/src/api.ts`, the UI in
`web/src/App.tsx` (reuse `FolderPicker`, `JobCard`, `.panel` / `.controls` / `.btn`); a new job kind also extends `Job["kind"]` and `JobCard`. 5. A new setting goes into
`config.rs` with a default, into `STARTER_CONFIG` (commented if optional) and `examples/config.toml`. 6. Test (below), update `README.md`, commit.
7. Report what was checked and what was NOT (a real YouTube download, a real external disk and the packaged window are the usual gaps).

## Build, test, install

```sh
. "$HOME/.cargo/env"                                   # cargo is often not on PATH in a fresh shell
cd web && npm install && npx tsc --noEmit && npm run build && cd ..    # the UI (must exist before the Rust build)
cargo test                                             # core, server and fetch tests
cd app && npm install && npx tauri build && cd ..      # the app: target/release/bundle/macos/CourseShelf.app
rm -rf /Applications/CourseShelf.app && cp -R target/release/bundle/macos/CourseShelf.app /Applications/ && rm -rf target/release/bundle/macos/CourseShelf.app
```
Quit the running app (Cmd-Q) before replacing it and tell the user to reopen it; keep ONE copy (a second one, even the build output, shows twice in Launchpad).
The folder that holds this repository may have a space in its name: quote paths.

**Test without real data or YouTube:** set `COURSESHELF_HOME=<scratch>/home` (a starter config is written there), or `HOME=<scratch>` for a true first start.
Never run `courseshelf-serve` or the app in a test without one of them: they would use the user's REAL data folder. For downloads, put a stand-in `yt-dlp` first on
PATH (it answers `-J` with JSON and writes a tiny mp4 with `ffmpeg -f lavfi` plus `.info.json`). `courseshelf-serve` prints a URL with `?t=<token>`: `curl -c jar` it
once, then call the API with `-b jar`. Headless Chrome (`--headless=new --virtual-time-budget=6000 --dump-dom "<url>#/manage"`) checks that a page renders.
A Finder launch: `env -i HOME=$HOME PATH=/usr/bin:/bin:/usr/sbin:/sbin <app>/Contents/MacOS/courseshelf-app` with `COURSESHELF_URL_FILE=<file>` (the app writes its address there).
Before sharing a version: a fresh clone, `npm install`, the build and `cargo test`, and a first start with a scratch `HOME`.

**Fast check of a viewer / UI change (about 2 minutes):**
1. `web`: `npx tsc --noEmit && npm run build`; `cargo build -p courseshelf-server` (the server embeds `web/dist` at compile time, so build the UI first).
2. Scratch data folder: run `COURSESHELF_HOME=<scratch>/home courseshelf-serve --help` once (it writes the starter config), then add a drive and a PDF folder,
   because the starter config has **no drive and `/api/books` answers 500 without one**:
   `[drives.local] path = "<scratch>/drive"` and `[pdf_sources] folders = ["<scratch>/books"]` with one sample PDF copied into `books/`.
3. `courseshelf-serve --port N --ui web/dist`, `curl -c jar "<printed url>"`, `curl -b jar .../api/books` gives the 16-hex id; the viewer is `<url>#/pdf/book/<id>`.
4. Headless Chrome screenshots the toolbar and panels but the PDF itself stays "Opening…"; to see pages or click, use the Chrome extension tools. Its key action
   does not send `\` (and other symbol keys may fail): test a key with `document.body.dispatchEvent(new KeyboardEvent('keydown', {key: '\\', bubbles: true}))`.
5. Stop the server and delete the scratch folder.

Pitfalls seen: a JSX attribute string is literal (`title="key \"`, not `"key \\"`); quit the installed app with `osascript -e 'tell application "CourseShelf" to quit'`
(check `pgrep -f "/Applications/CourseShelf.app/Contents/MacOS"` first); after installing, `grep -a -c "<new UI text>" /Applications/CourseShelf.app/Contents/MacOS/courseshelf-app`
confirms the new UI is inside.

## Making a book from a list of papers (scripted)

What the Papers tab's "Make a book" does, for a list given in chat (only when the user asks for that book; it writes to their data folder):
1. **IDs.** A database paper has a 10-hex id (`GET /api/papers` -> `papers[].id`; match on `arxiv`, never on `sno`). A PDF from a `[pdf_sources]` folder has
   a 16-hex id (`GET /api/books` -> `books[].id`). A book may mix both, in any order.
2. **A cover or contents page first**: typeset it (e.g. pdflatex) into a folder listed in `[pdf_sources]` and put its 16-hex id first. Its title in the book
   comes from the file name (leading digits and `_` stripped), so name it `00_<Title_Words>.pdf`. To add such a folder, back up `config.toml` first and add one line.
3. **Create**: `POST /api/collections/create {"title": ..., "paper_ids": [...]}` -> `{"job": N}` (422 with a plain message if the title exists, the list is empty or another job is running).
   Papers that are not on disk are downloaded to `papers/inbox/` (3 s apart). `POST /api/collections/add {"id", "paper_ids"}` appends to an existing book.
4. **Wait**: `GET /api/manage/jobs` returns `{"jobs": [...]}` (a dict, not a list); poll until the job's `status` is `done`, `failed` or `cancelled`, then read its `warnings`.
5. **Check** `collections.json` (each paper's `start` and `pages`) and `pdfinfo collections/<id>.pdf`: the page count must equal the last start + pages - 1.
A separate `courseshelf-serve` on the same data folder works while the app is open (the app re-reads `collections.json` and `config.toml` on each request), but the
book lock is per process: do not let the user make a book in the app at the same moment. Stop the server afterwards.

## Code that differs per operating system

The core must compile on macOS, Windows and Linux (the workflow checks all three). Keep OS-specific code behind `#[cfg(unix)]` / `#[cfg(windows)]` with a
version for each, and never use `std::os::unix`, `libc` or a macOS-only command unguarded. Today: free space (`config::disk_space`: statvfs / GetDiskFreeSpaceExW,
via the target-specific `libc` / `windows-sys` dependencies), the same-disk check (`mover::dev`: device number / drive letter), the home folder
(`config::home_dir`: HOME, else USERPROFILE), external drives (`Drive::is_external`: /Volumes, /media, /mnt or a drive letter that is not %SystemDrive%;
`Drive::mount_point` is the drive letter on Windows), opening files (`tools::open`) and the folder browser's start (/Volumes, the list of drive letters, /media).
Tests that need a Unix tool (`sh`, `ls`) are `#[cfg(unix)]`; build test paths with an absolute root of the OS (`C:/` on Windows: `/lib` is not absolute there).
Check Windows from the Mac before pushing (MinGW from Homebrew, `rustup target add x86_64-pc-windows-gnu`):
`cargo test --workspace --exclude courseshelf-app --target x86_64-pc-windows-gnu --no-run` and `cargo check -p courseshelf-app --target x86_64-pc-windows-gnu`.
This compiles but does not run the Windows tests: the workflow runs them. The app needs `icons/icon.ico` on Windows (made from `icon.png`).

## Things that went wrong before (do not repeat)

- **"yt-dlp was not found" from Finder**: a Finder-launched app gets launchd's bare PATH. `tools.rs` also searches `/opt/homebrew/bin`, `/usr/local/bin`,
  `~/.local/bin`, `~/.cargo/bin` and gives children the extended PATH. `open` from a terminal does NOT reproduce a Finder launch (see the test above).
- **Settings lost on every restart** when kept in browser storage (random port): keep them server-side.
- **A same-disk move left the original behind; progress went to the wrong database**: fixed in `mover.rs` / `progress.rs`; keep their tests.
- **Two app copies in Launchpad**: see the install step.
- **The core did not compile on Windows** (`std::os::unix` in mover.rs, `libc::statvfs` in config.rs) while the README said it did; nothing built it there.
  Fixed with the per-OS code above, and the workflow now builds every OS on each push.
- **A course listed twice, with its progress split in two**: a course's location (locations.json) pointed INSIDE the Mac drive under another folder name
  (`<drive>/Extra/Provider/Course`), so the drive scan listed that folder again as an unlisted course, and each entry saved progress to its own database.
  Now both tabs skip a scanned folder that is a course's location, the location's copy is labelled with the drive it lies in (and as the Mac copy), and
  `build_state` also reads progress saved under the folder's own name (`CopyRef::alias`). Test: `located_course_inside_a_drive_is_listed_once`.
- **A single video is not a course until it is inside a drive**: a course is a FOLDER with videos, up to 3 levels under a drive root.
- yt-dlp: YouTube sometimes answers 429 for subtitles (the lecture is kept); `--embed-subs` needs ffmpeg; VP9/AV1 may not play in the app's video engine,
  so H.264 wins only as a tie-breaker at equal resolution (`-S "res,vcodec:h264,acodec:aac"`).

## Data files (in the data folder, next to config.toml)

| File | Owner | Notes |
|---|---|---|
| `config.toml` | the user | the app edits only a drive's `path` line, and re-validates before replacing the file |
| `courses.csv` | the user | never rewritten by the app; `import-plans` fills it only when it is empty (or with `--force`) |
| `locations.json` | app | course -> folder when not at `<drive>/<course>` |
| `singles.json` | app | saved single-video links, status, last folder |
| `ui-settings.json` | app | font size, theme, autoplay-next |
| `collections.json`, `collections/<id>.pdf` | app | books joined from papers; their marks in `annotations/<id>.json` |
| `bookmarks.json` | app | Books-tab bookmarks (full PDF paths) |
| `annotations/`, `papers/inbox/`, `cache/` | app | PDF marks, downloaded papers, playlist listings |
| `papers/papers.csv`, `papers/backups/` | app (Add papers) | the starter paper list (only without `[papers]`) and the backups made before each append |
| `<drive>/.courseshelf/progress.sqlite` | app | watch progress, travels with the disk |

An unplugged disk shows "not connected"; never create folders on a missing disk (the folder browser refuses when the parent does not exist).

## Git and releases

- This folder is the repository (`main`, remote `origin`). Commit after every checked change with a message that says what and why (attribution lines of the session
  at the end). Push to `origin` after every change that is verified (tests pass, the new commits scanned clean): the owner asked for this on 2026-10-24; push `main` only; never rewrite pushed history. Scan new commits for personal paths and data before a push.
- Tracked: `crates/`, `app/` (without `node_modules`), `web/src`, `web/public`, `examples/`, README, CLAUDE.md, LICENSE. Never: build output, `node_modules`, user data, `*.local.md`.
- A release: raise `version` in `app/src-tauri/tauri.conf.json`, describe it in `README.md`, run the fresh-clone test, commit, tag `vX.Y.Z`.
- **GitHub Actions** (`.github/workflows/build.yml`): on every push to `main`, tag `v*`, pull request or manual run, on macOS, Windows and Linux: `npm ci`, `tsc`,
  UI build, `cargo test --workspace --exclude courseshelf-app`, the release downloader, `npx tauri build --bundles app|nsis|deb`; the app and downloader are
  kept as run artifacts `CourseShelf-<OS>` (unsigned). Linux needs the webkit2gtk-4.1 / GTK packages installed in the workflow. Look at the run after a push
  (`/repos/.../actions/runs?branch=main`, read with the token header file); a red run is a broken change. Pushing a change to a file in `.github/workflows/`
  needs a token with the `workflow` scope (a `repo`-only token is refused).
- There is no self-updater or GitHub Release yet: users build from source or take a run's artifacts. The plan (like The Data Application's updater, VLC-style):
  Tauri's updater plugin, a signing key pair (the private key as a repository secret), and a release job in the workflow that publishes a signed release for each
  tag; the app checks `releases/latest` and offers Install and restart.
